//! Durable target observations, allocation and operator repair.

use crate::{
    ports::{
        BindingEvidence, DurableStructuralProof, DurableWorkAdmission, EvidenceKind,
        ExecutionEvidence, GuardedInvalidationTransition, GuardedSeatTransition,
        HostInvalidationFence, HostInvalidationReason, HostObservation, HostObservationAdmission,
        HostUiState, InvalidationSeatPage, ObservationProvenance, OperatorRequest,
        OrdinaryResolutionAttempt, OrdinaryResolutionGuard, OrdinaryResolutionOutcome,
        PriorPublishedTarget, PublishedSnapshot, ReconciliationAction, ReconciliationOutcome,
        ResolvedTargetCheck, SeatState, SnapshotCleanupProgress, SnapshotGenerationId,
        SnapshotHeader, SnapshotSavedSeat, SnapshotSeatPage, SnapshotStage, SnapshotStageProgress,
        SnapshotTargetMatch, StructuralOccupancy, UnresolvedReason,
    },
    protocol::{
        authority::{
            CARRIED_BINDING_PROVENANCES, DecisionFence, MutationPermit, ObligationRef,
            OperatorActor,
        },
        commands::CheckIn,
        ids::{ExecutionId, HostBootId, HostTargetId, SeatId, TerminalId, prefix},
        results::{ApiError, CheckInResult, CommandResult, ErrorCode},
        time::{CallBudget, UtcMillis},
    },
    store::{
        catch_up,
        connection::{DecisionInstant, StoreContext, api_error, store_error},
        control,
        effective::{self, EffectiveObservationSource, EffectiveRecoveryDisposition},
        schema,
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

#[cfg(test)]
use crate::ports::HostSnapshot;

#[cfg(test)]
fn provenance_name(value: ObservationProvenance) -> &'static str {
    match value {
        ObservationProvenance::FreshCurrentTarget => "fresh",
        ObservationProvenance::CoherentEnumeration => "enumeration",
        ObservationProvenance::UncharacterizedCache => "cache",
    }
}

fn checked_host_number(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value).map_err(|_| {
        api_error(
            ErrorCode::InvalidRequest,
            "host evidence exceeds SQLite integer range",
        )
    })
}

fn snapshot_budget(context: &StoreContext, budget: &CallBudget) -> Result<(), ApiError> {
    if budget.cancellation.is_cancelled() {
        Err(api_error(ErrorCode::Cancelled, "snapshot work cancelled"))
    } else if context.clock().monotonic_now() >= budget.deadline {
        Err(api_error(
            ErrorCode::DeadlineExceeded,
            "snapshot work budget exhausted",
        ))
    } else {
        Ok(())
    }
}

/// Columns of a `snapshot_generations` row, in `load_snapshot_stage`'s SELECT order.
type SnapshotStageColumns = (String, String, i64, i64, String, i64, i64, String);

fn load_snapshot_stage(
    conn: &Connection,
    stage: &SnapshotGenerationId,
) -> Result<SnapshotStageColumns, ApiError> {
    conn.query_row(
        "SELECT instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status FROM snapshot_generations WHERE id=?1",
        [stage.as_str()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
    )
    .optional()
    .map_err(store_error)?
    .ok_or_else(|| api_error(ErrorCode::NotFound, "snapshot stage not found"))
}

fn snapshot_stage(
    stage: &SnapshotGenerationId,
    expected: i64,
    staged: i64,
    status: &str,
) -> SnapshotStage {
    SnapshotStage {
        id: stage.clone(),
        expected_targets: expected as u64,
        staged_targets: staged as u64,
        sealed: status == "sealed" || status == "published",
    }
}

pub fn begin_host_observation(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    budget: &CallBudget,
) -> Result<HostObservationAdmission, ApiError> {
    snapshot_budget(context, budget)?;
    if instance.is_empty() || instance.len() > 128 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid host instance",
        ));
    }
    context.execute_budgeted_decision(
        conn, budget,
        |_| Ok(()),
        |tx, at, ()| {
            tx.execute(
                "INSERT INTO host_instances(id,created_at) VALUES (?1,?2) ON CONFLICT(id) DO NOTHING",
                params![instance, at.utc.0],
            )
            .map_err(store_error)?;
            let changed = tx
                .execute(
                    "UPDATE host_instances SET observation_admission_sequence=observation_admission_sequence+1 WHERE id=?1 AND observation_admission_sequence<?2",
                    params![instance, i64::MAX],
                )
                .map_err(store_error)?;
            if changed != 1 {
                return Err(api_error(ErrorCode::SequenceExhausted, "host observation admission exhausted"));
            }
            let (sequence, active, boot, epoch, lifecycle, invalidation): (
                i64, Option<String>, Option<String>, i64, i64, i64,
            ) = tx
                .query_row(
                    "SELECT observation_admission_sequence,active_snapshot_id,host_boot,host_epoch,lifecycle_revision,invalidation_revision FROM host_instances WHERE id=?1",
                    [instance],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .map_err(store_error)?;
            Ok(HostObservationAdmission {
                instance: instance.into(),
                sequence: sequence as u64,
                expected_active: active.map(SnapshotGenerationId::store_issued),
                expected_boot: boot.map(HostBootId::new),
                expected_epoch: epoch as u64,
                lifecycle_revision: lifecycle as u64,
                invalidation_revision: invalidation as u64,
            })
        },
    )
}

pub fn persisted_host_epoch(conn: &Connection, instance: &str) -> Result<u64, ApiError> {
    let epoch: Option<i64> = conn
        .query_row(
            "SELECT host_epoch FROM host_instances WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    u64::try_from(epoch.unwrap_or(0))
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative persisted host epoch"))
}

/// Commit one explicit current-target read under the store-issued observation
/// admission. A late or superseded host call never rewrites a newer row.
pub fn publish_current_target_observation(
    context: &StoreContext,
    conn: &mut Connection,
    admission: &HostObservationAdmission,
    observation: &HostObservation,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    snapshot_budget(context, budget)?;
    if !observation.is_fresh_structure()
        || observation.target.as_str().is_empty()
        || observation.host_boot.as_str().is_empty()
        || observation.observation_sequence == 0
    {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "target read lacks fresh structural order",
        ));
    }
    let admission_sequence = checked_host_number(admission.sequence)?;
    let host_epoch = checked_host_number(observation.epoch)?;
    let generation = checked_host_number(observation.generation)?;
    let sequence = checked_host_number(observation.observation_sequence)?;
    let connection_epoch = checked_host_number(observation.connection_epoch)?;
    let proof = observation.verified_structural_proof();
    let occupancy = match observation.occupancy {
        StructuralOccupancy::EmptyShell => "empty_shell",
        StructuralOccupancy::Occupied => "occupied",
        StructuralOccupancy::Unknown => "unknown",
    };
    let ui = match observation.ui {
        HostUiState::Idle => "idle",
        HostUiState::ActiveTurn => "active_turn",
        HostUiState::ApprovalOrQuestion => "approval_or_question",
        HostUiState::HumanInput => "human_input",
        HostUiState::Unknown => "unknown",
    };
    let top_level = observation
        .occupant
        .as_ref()
        .is_some_and(|occupant| occupant.is_top_level);
    let execution = match &observation.execution {
        ExecutionEvidence::Verified { execution, .. } => Some(execution.as_str()),
        ExecutionEvidence::Unknown => None,
    };
    context.execute_budgeted_decision(conn, budget,
        |tx| {
            type HostColumns = Option<(Option<String>, i64, i64, Option<String>, i64, i64, i64, i64)>;
            let host: HostColumns = tx.query_row(
                "SELECT host_boot,host_epoch,observation_sequence,active_snapshot_id,lifecycle_revision,invalidation_revision,observation_admission_sequence,observation_decided_sequence FROM host_instances WHERE id=?1",
                [admission.instance()],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)),
            ).optional().map_err(store_error)?;
            let Some((boot, epoch, published_sequence, active, lifecycle, invalidation, issued, decided)) = host else {
                return Ok(false);
            };
            if admission_sequence > issued
                || admission_sequence <= decided
                || admission.expected_boot.as_ref().map(HostBootId::as_str) != boot.as_deref()
                || checked_host_number(admission.expected_epoch)? != epoch
                || admission.expected_active.as_ref().map(SnapshotGenerationId::as_str) != active.as_deref()
                || checked_host_number(admission.lifecycle_revision)? != lifecycle
                || checked_host_number(admission.invalidation_revision)? != invalidation
                || boot.as_deref() != Some(observation.host_boot.as_str())
                || epoch != host_epoch
                || sequence <= published_sequence
            {
                return Ok(false);
            }
            let Some(active) = active else { return Ok(false); };
            if let Some(proof) = &proof {
                let snapshot_incarnation: Option<String> = tx.query_row(
                    "SELECT incarnation FROM snapshot_generations WHERE id=?1 AND instance_id=?2 AND status='published'",
                    params![active,admission.instance()],
                    |r| r.get(0),
                ).optional().map_err(store_error)?;
                if snapshot_incarnation.as_deref() != Some(proof.incarnation()) {
                    return Ok(false);
                }
            }
            let prior: Option<(String, i64, i64, Option<i64>)> = tx.query_row(
                "SELECT host_boot,epoch,observation_sequence,connection_epoch FROM observed_targets WHERE instance_id=?1 AND target_id=?2",
                params![admission.instance(),observation.target.as_str()],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
            ).optional().map_err(store_error)?;
            if prior.is_some_and(|(prior_boot,prior_epoch,prior_sequence,prior_connection)| {
                prior_boot == observation.host_boot.as_str()
                    && prior_epoch == host_epoch
                    && (prior_sequence >= sequence || prior_connection.is_some_and(|prior| prior > connection_epoch))
            }) {
                return Ok(false);
            }
            Ok(true)
        },
        |tx, _, fresh| {
            if !fresh { return Ok(false); }
            tx.execute(
                "INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,connection_epoch,incarnation,incarnation_source_kind,observed_at,provenance,terminal_id,occupancy,ui_state,verified_execution,top_level_occupant) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'fresh',?11,?12,?13,?14,?15) ON CONFLICT(instance_id,target_id) DO UPDATE SET host_boot=excluded.host_boot,epoch=excluded.epoch,generation=excluded.generation,observation_sequence=excluded.observation_sequence,connection_epoch=excluded.connection_epoch,incarnation=excluded.incarnation,incarnation_source_kind=excluded.incarnation_source_kind,observed_at=excluded.observed_at,provenance='fresh',terminal_id=excluded.terminal_id,occupancy=excluded.occupancy,ui_state=excluded.ui_state,verified_execution=excluded.verified_execution,top_level_occupant=excluded.top_level_occupant",
                params![admission.instance(),observation.target.as_str(),observation.host_boot.as_str(),host_epoch,generation,sequence,
                    proof.as_ref().map(|_| connection_epoch),proof.as_ref().map(DurableStructuralProof::incarnation),
                    proof.as_ref().map(DurableStructuralProof::source_spelling),observation.observed_at_utc.0,
                    observation.terminal.as_ref().map(TerminalId::as_str),occupancy,ui,execution,top_level],
            ).map_err(store_error)?;
            tx.execute(
                "UPDATE host_instances SET observation_decided_sequence=?1 WHERE id=?2 AND observation_decided_sequence<?1",
                params![admission_sequence,admission.instance()],
            ).map_err(store_error)?;
            let current_seat: Option<(String, bool)> = tx.query_row(
                "SELECT s.id, s.target_generation=?3 AND EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=s.id AND b.ended_at IS NULL AND b.registered_at IS NOT NULL AND b.target_id=s.target_id AND b.generation=s.generation AND b.target_generation=s.target_generation AND b.host_boot=?4 AND b.host_epoch=?5) FROM seats s WHERE s.instance_id=?1 AND s.target_id=?2 AND s.state='resolved'",
                params![admission.instance(),observation.target.as_str(),generation,observation.host_boot.as_str(),host_epoch],
                |r| Ok((r.get(0)?,r.get(1)?)),
            ).optional().map_err(store_error)?;
            if let Some((seat, false)) = current_seat {
                schema::ensure_unavailability_episode(tx, &SeatId::new(seat))?;
            }
            schema::apply_eligibility_transition(tx, admission.instance(), |_| Ok(true))?;
            schema::bump_lifecycle_revision(tx, admission.instance())?;
            Ok(true)
        },
    )
}

pub fn invalidate_host_observation(
    context: &StoreContext,
    conn: &mut Connection,
    admission: &HostObservationAdmission,
    _reason: HostInvalidationReason,
    budget: &CallBudget,
) -> Result<Option<HostInvalidationFence>, ApiError> {
    snapshot_budget(context, budget)?;
    let sequence = checked_host_number(admission.sequence)?;
    context.execute_budgeted_decision(
        conn, budget,
        |tx| {
            let state: Option<(i64, i64)> = tx
                .query_row(
                    "SELECT observation_admission_sequence,observation_decided_sequence FROM host_instances WHERE id=?1",
                    [admission.instance()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            let Some((issued, decided)) = state else {
                return Err(api_error(ErrorCode::NotFound, "host instance not found"));
            };
            Ok(sequence <= issued && sequence > decided)
        },
        |tx, _, fresh| {
            if !fresh {
                return Ok(None);
            }
            let changed = tx
                .execute(
                    "UPDATE host_instances SET observation_decided_sequence=?1,invalidation_revision=invalidation_revision+1 WHERE id=?2 AND observation_decided_sequence<?1 AND invalidation_revision<?3",
                    params![sequence, admission.instance(), i64::MAX],
                )
                .map_err(store_error)?;
            if changed != 1 {
                return Err(api_error(ErrorCode::SequenceExhausted, "host invalidation revision exhausted"));
            }
            schema::apply_eligibility_transition(tx, admission.instance(), |_| Ok(true))?;
            let revision: i64 = tx
                .query_row(
                    "SELECT invalidation_revision FROM host_instances WHERE id=?1",
                    [admission.instance()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            Ok(Some(HostInvalidationFence {
                admission: admission.clone(),
                invalidation_revision: revision as u64,
            }))
        },
    )
}

pub fn begin_snapshot_stage(
    context: &StoreContext,
    conn: &mut Connection,
    header: SnapshotHeader,
    budget: &CallBudget,
) -> Result<SnapshotStage, ApiError> {
    snapshot_budget(context, budget)?;
    let epoch = checked_host_number(header.epoch)?;
    let sequence = checked_host_number(header.observation_sequence)?;
    let expected = checked_host_number(header.expected_targets)?;
    let admission_sequence = checked_host_number(header.admission.sequence)?;
    if header.instance.is_empty()
        || header.instance.len() > 128
        || header.admission.instance != header.instance
        || header.boot.as_str().is_empty()
        || header.incarnation.is_empty()
        || header.incarnation.len() > 128
        || epoch <= 0
        || sequence <= 0
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid snapshot header",
        ));
    }
    let id = SnapshotGenerationId::store_issued(format!("snapshot-{}", uuid::Uuid::new_v4()));
    context.execute_budgeted_decision(
        conn, budget,
        |tx| {
            type OldColumns = Option<(Option<String>, i64, i64, Option<String>, i64, i64, Option<i64>, i64, i64)>;
            let old: OldColumns = tx
                .query_row(
                    "SELECT host_boot,host_epoch,observation_sequence,active_snapshot_id,lifecycle_revision,invalidation_revision,recovery_epoch,observation_admission_sequence,observation_decided_sequence FROM host_instances WHERE id=?1",
                    [&header.instance],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)),
                )
                .optional()
                .map_err(store_error)?;
            if let Some((Some(boot), current_epoch, current_sequence, _, _, _, _, _, _)) = &old
                && boot == header.boot.as_str()
                    && (*current_epoch > epoch
                        || (*current_epoch == epoch && *current_sequence >= sequence))
                {
                    return Err(api_error(ErrorCode::StaleHostObservation, "snapshot order is stale"));
                }
            let Some((saved_boot, saved_epoch, _, active, lifecycle, invalidation, _, issued, decided)) = &old else {
                return Err(api_error(ErrorCode::StaleHostObservation, "host admission is missing"));
            };
            if admission_sequence > *issued || admission_sequence <= *decided
                || active.as_deref() != header.admission.expected_active.as_ref().map(|id| id.as_str())
                || saved_boot.as_deref() != header.admission.expected_boot.as_ref().map(|id| id.as_str())
                || *saved_epoch != checked_host_number(header.admission.expected_epoch)?
                || *lifecycle != header.admission.lifecycle_revision as i64
                || *invalidation != header.admission.invalidation_revision as i64
            {
                return Err(api_error(ErrorCode::StaleHostObservation, "host admission changed"));
            }
            Ok(old)
        },
        |tx, at, old| {
            let (active, lifecycle, invalidation, recovery_epoch) = old
                .map(|(_, _, _, active, lifecycle, invalidation, recovery, _, _)| {
                    (active, lifecycle, invalidation, recovery)
                })
                .unwrap_or((None, 0, 0, None));
            tx.execute(
                "INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,status,captured_active_id,captured_lifecycle_revision,captured_invalidation_revision,captured_recovery_epoch,admission_sequence,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,'building',?8,?9,?10,?11,?12,?13)",
                params![id.as_str(), header.instance, header.boot.as_str(), epoch, sequence, header.incarnation, expected, active, lifecycle, invalidation, recovery_epoch, admission_sequence, at.utc.0],
            )
            .map_err(store_error)?;
            Ok(snapshot_stage(&id, expected, 0, "building"))
        },
    )
}

pub fn stage_snapshot_targets(
    context: &StoreContext,
    conn: &mut Connection,
    stage: &SnapshotGenerationId,
    offset: u64,
    targets: &[HostObservation],
    admission: DurableWorkAdmission,
    budget: &CallBudget,
) -> Result<SnapshotStageProgress, ApiError> {
    snapshot_budget(context, budget)?;
    if targets.is_empty()
        || targets.len() > usize::from(admission.max_units)
        || admission.max_units > 16
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "snapshot slice exceeds admission",
        ));
    }
    let offset = checked_host_number(offset)?;
    let mut checked = Vec::with_capacity(targets.len());
    for target in targets {
        if target.provenance == ObservationProvenance::UncharacterizedCache
            || target.observation_sequence == 0
        {
            return Err(api_error(
                ErrorCode::StaleHostObservation,
                "untrusted snapshot target",
            ));
        }
        checked.push((
            checked_host_number(target.epoch)?,
            checked_host_number(target.generation)?,
            checked_host_number(target.observation_sequence)?,
        ));
    }
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(store_error)?;
    // The quantum bounds how long this turn holds the writer, so it starts
    // once the write lock is held: waiting for the lock is not holding it.
    let started = context.clock().monotonic_now();
    let (_, boot, epoch, sequence, incarnation, expected, staged, status) =
        load_snapshot_stage(&tx, stage)?;
    if status != "building" || staged != offset || staged + targets.len() as i64 > expected {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "snapshot slice is not contiguous",
        ));
    }
    // A spent quantum ends the slice with its committed prefix (at least one
    // target, so every turn progresses); the caller stages the rest in the
    // next turn. A scheduling stall is not a failed capture: failing here
    // would invalidate the host and end bindings on no host evidence.
    let mut done = 0usize;
    for (target, (target_epoch, generation, target_sequence)) in targets.iter().zip(checked) {
        snapshot_budget(context, budget)?;
        if done > 0 && context.clock().monotonic_now().0.saturating_sub(started.0) >= 5 {
            break;
        }
        if target.host_boot.as_str() != boot || target_epoch != epoch || target_sequence > sequence
        {
            return Err(api_error(
                ErrorCode::StaleHostObservation,
                "snapshot target order differs",
            ));
        }
        let occupancy = match target.occupancy {
            StructuralOccupancy::EmptyShell => "empty_shell",
            StructuralOccupancy::Occupied => "occupied",
            StructuralOccupancy::Unknown => "unknown",
        };
        let ui = match target.ui {
            HostUiState::Idle => "idle",
            HostUiState::ActiveTurn => "active_turn",
            HostUiState::ApprovalOrQuestion => "approval_or_question",
            HostUiState::HumanInput => "human_input",
            HostUiState::Unknown => "unknown",
        };
        let execution = match &target.execution {
            ExecutionEvidence::Verified { execution, .. } => Some(execution.as_str()),
            ExecutionEvidence::Unknown => None,
        };
        let top_level = target
            .occupant
            .as_ref()
            .is_some_and(|occupant| occupant.is_top_level);
        let (connection_epoch, incarnation_source) = if target.terminal.is_some() {
            match target.provenance {
                ObservationProvenance::CoherentEnumeration => {
                    let matching_incarnation = match &target.incarnation {
                        crate::ports::IncarnationEvidence::Unknown => true,
                        crate::ports::IncarnationEvidence::Verified {
                            identity,
                            evidence_kind: EvidenceKind::CoherentEnumeration,
                        } => identity == &incarnation,
                        _ => false,
                    };
                    if !matching_incarnation {
                        return Err(api_error(
                            ErrorCode::StaleHostObservation,
                            "snapshot target incarnation conflicts with coherent capture",
                        ));
                    }
                    (
                        Some(checked_host_number(target.connection_epoch)?),
                        Some("coherent_enumeration"),
                    )
                }
                ObservationProvenance::FreshCurrentTarget => {
                    match target.verified_structural_proof() {
                        Some(proof) if proof.incarnation() == incarnation => (
                            Some(checked_host_number(proof.connection_epoch())?),
                            Some(proof.source_spelling()),
                        ),
                        _ => (None, None),
                    }
                }
                ObservationProvenance::UncharacterizedCache => unreachable!(),
            }
        } else {
            (None, None)
        };
        let result = tx.execute(
            "INSERT INTO snapshot_targets(generation_id,target_id,terminal_id,generation,observation_sequence,connection_epoch,incarnation_source_kind,occupancy,ui_state,observed_at,verified_execution,top_level_occupant) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![stage.as_str(), target.target.as_str(), target.terminal.as_ref().map(|id| id.as_str()), generation, target_sequence, connection_epoch, incarnation_source, occupancy, ui, target.observed_at_utc.0, execution, top_level],
        );
        if let Err(error) = result {
            if matches!(error, rusqlite::Error::SqliteFailure(_, _)) {
                return Err(api_error(
                    ErrorCode::InvalidRequest,
                    "duplicate or invalid snapshot target",
                ));
            }
            return Err(store_error(error));
        }
        done += 1;
    }
    let staged = staged + done as i64;
    tx.execute(
        "UPDATE snapshot_generations SET staged_targets=?1 WHERE id=?2 AND status='building'",
        params![staged, stage.as_str()],
    )
    .map_err(store_error)?;
    tx.commit().map_err(store_error)?;
    Ok(SnapshotStageProgress {
        stage: snapshot_stage(stage, expected, staged, "building"),
        visited: done as u8,
    })
}

pub fn seal_snapshot_stage(
    context: &StoreContext,
    conn: &mut Connection,
    stage: &SnapshotGenerationId,
    budget: &CallBudget,
) -> Result<SnapshotStage, ApiError> {
    snapshot_budget(context, budget)?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let (_, _, _, _, _, expected, staged, status) = load_snapshot_stage(&tx, stage)?;
    if staged != expected {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "snapshot target set incomplete",
        ));
    }
    if status == "building" {
        tx.execute(
            "UPDATE snapshot_generations SET status='sealed' WHERE id=?1",
            [stage.as_str()],
        )
        .map_err(store_error)?;
    } else if status != "sealed" {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "snapshot cannot be sealed",
        ));
    }
    tx.commit().map_err(store_error)?;
    Ok(snapshot_stage(stage, expected, staged, "sealed"))
}

pub fn publish_snapshot_stage(
    context: &StoreContext,
    conn: &mut Connection,
    stage: &SnapshotGenerationId,
    budget: &CallBudget,
) -> Result<PublishedSnapshot, ApiError> {
    snapshot_budget(context, budget)?;
    context.execute_budgeted_decision(
        conn, budget,
        |tx| {
            type RowColumns = Option<(
                String, String, i64, i64, String, i64, i64, String,
                Option<String>, i64, i64, Option<i64>, i64,
            )>;
            let row: RowColumns = tx
                .query_row(
                    "SELECT instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_active_id,captured_lifecycle_revision,captured_invalidation_revision,captured_recovery_epoch,admission_sequence FROM snapshot_generations WHERE id=?1",
                    [stage.as_str()],
                    |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?)),
                )
                .optional()
                .map_err(store_error)?;
            let Some((instance, boot, epoch, sequence, incarnation, expected, staged, status, captured_active, captured_lifecycle, captured_invalidation, captured_recovery_epoch, admission_sequence)) = row else {
                return Err(api_error(ErrorCode::NotFound, "snapshot stage not found"));
            };
            if status != "sealed" || staged != expected {
                return Err(api_error(ErrorCode::StaleHostObservation, "snapshot stage is incomplete"));
            }
            type CurrentColumns = (
                Option<String>, Option<String>, i64, i64, Option<String>, Option<i64>,
                Option<String>, i64, i64, i64, i64, i64,
            );
            let current: CurrentColumns = tx
                .query_row(
                    "SELECT active_snapshot_id,host_boot,host_epoch,observation_sequence,recovery_boot,recovery_epoch,recovery_baseline_generation_id,baseline_hold_unclaimed,lifecycle_revision,invalidation_revision,observation_admission_sequence,observation_decided_sequence FROM host_instances WHERE id=?1",
                    [&instance],
                    |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?)),
                )
                .map_err(store_error)?;
            let (active, old_boot, old_epoch, old_sequence, recovery_boot, recovery_epoch, baseline, old_hold, lifecycle, invalidation, issued, decided) = current;
            if active != captured_active
                || lifecycle != captured_lifecycle
                || invalidation != captured_invalidation
                || recovery_epoch != captured_recovery_epoch
                || admission_sequence <= decided
                || admission_sequence > issued
                || old_boot.as_deref() == Some(boot.as_str())
                    && (old_epoch > epoch || old_epoch == epoch && old_sequence >= sequence)
            {
                return Err(api_error(ErrorCode::StaleHostObservation, "snapshot publication fence changed"));
            }
            let prior_incarnation: Option<String> = match active.as_deref() {
                Some(id) => tx
                    .query_row(
                        "SELECT incarnation FROM snapshot_generations WHERE id=?1 AND status='published'",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(store_error)?,
                None => None,
            };
            let changed_or_unknown_incarnation = prior_incarnation
                .as_deref()
                .is_none_or(|prior| prior != incarnation);
            let nonretired = has_nonretired_seat(tx, &instance)?;
            let unresolved: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM seats WHERE instance_id=?1 AND state='unresolved' LIMIT 1)",
                    [&instance],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let new_recovery = recovery_boot.as_deref() != Some(boot.as_str())
                || recovery_epoch != Some(epoch);
            let hold = unresolved || changed_or_unknown_incarnation && nonretired
                || !new_recovery && old_hold != 0;
            let baseline = match baseline {
                Some(baseline) if !new_recovery => baseline,
                _ => stage.as_str().to_owned(),
            };
            Ok((instance, boot, epoch, sequence, incarnation, expected, admission_sequence, invalidation, new_recovery, baseline, hold))
        },
        |tx, _, (instance, boot, epoch, sequence, incarnation, expected, admission_sequence, invalidation, _new_recovery, baseline, hold)| {
            let changed = tx
                .execute(
                    "UPDATE snapshot_generations SET status='published',published_invalidation_revision=?1 WHERE id=?2 AND status='sealed' AND staged_targets=expected_targets",
                    params![invalidation, stage.as_str()],
                )
                .map_err(store_error)?;
            if changed != 1 {
                return Err(api_error(ErrorCode::StaleHostObservation, "snapshot stage changed"));
            }
            tx.execute(
                "UPDATE host_instances SET host_boot=?1,host_epoch=?2,observation_sequence=?3,recovery_boot=?1,recovery_epoch=?2,active_snapshot_id=?4,recovery_baseline_generation_id=?5,baseline_hold_unclaimed=?6,observation_decided_sequence=?7 WHERE id=?8",
                params![boot,epoch,sequence,stage.as_str(),baseline,hold,admission_sequence,instance],
            )
            .map_err(store_error)?;
            schema::apply_eligibility_transition(tx, &instance, |_| Ok(true))?;
            Ok(PublishedSnapshot {
                id: stage.clone(),
                instance,
                boot: HostBootId::new(boot),
                epoch: epoch as u64,
                observation_sequence: sequence as u64,
                incarnation,
                target_count: expected as u64,
                invalidation_revision: invalidation as u64,
            })
        },
    )
}

pub(crate) fn has_nonretired_seat(conn: &Connection, instance: &str) -> Result<bool, ApiError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND state!='retired' LIMIT 1)",
        [instance],
        |r| r.get(0),
    )
    .map_err(store_error)
}

fn active_publication(
    conn: &Connection,
    stage: &SnapshotGenerationId,
) -> Result<PublishedSnapshot, ApiError> {
    type RowColumns = Option<(String, String, i64, i64, String, i64, i64, i64)>;
    let row: RowColumns = conn
        .query_row(
            "SELECT g.instance_id,g.host_boot,g.epoch,g.observation_sequence,g.incarnation,g.expected_targets,g.published_invalidation_revision,h.invalidation_revision FROM snapshot_generations g JOIN host_instances h ON h.id=g.instance_id AND h.active_snapshot_id=g.id WHERE g.id=?1 AND g.status='published'",
            [stage.as_str()],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((
        instance,
        boot,
        epoch,
        sequence,
        incarnation,
        count,
        published_invalidation,
        current_invalidation,
    )) = row
    else {
        return Err(api_error(
            ErrorCode::CursorStale,
            "published snapshot changed",
        ));
    };
    if current_invalidation != published_invalidation {
        return Err(api_error(
            ErrorCode::CursorStale,
            "published snapshot invalidated",
        ));
    }
    Ok(PublishedSnapshot {
        id: stage.clone(),
        instance,
        boot: HostBootId::new(boot),
        epoch: epoch as u64,
        observation_sequence: sequence as u64,
        incarnation,
        target_count: count as u64,
        invalidation_revision: current_invalidation as u64,
    })
}

fn snapshot_match(
    conn: &Connection,
    stage: &SnapshotGenerationId,
    target: Option<&str>,
    terminal: Option<&str>,
) -> Result<Option<SnapshotTargetMatch>, ApiError> {
    type RowColumns = Option<(
        String,
        Option<String>,
        i64,
        i64,
        Option<i64>,
        Option<String>,
        String,
        Option<String>,
        i64,
    )>;
    let row: RowColumns =
        if let Some(terminal) = terminal {
            conn.query_row(
                "SELECT target_id,terminal_id,generation,observation_sequence,connection_epoch,incarnation_source_kind,occupancy,verified_execution,top_level_occupant FROM snapshot_targets WHERE generation_id=?1 AND terminal_id=?2",
                params![stage.as_str(),terminal],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
            )
        } else if let Some(target) = target {
            conn.query_row(
                "SELECT target_id,terminal_id,generation,observation_sequence,connection_epoch,incarnation_source_kind,occupancy,verified_execution,top_level_occupant FROM snapshot_targets WHERE generation_id=?1 AND target_id=?2",
                params![stage.as_str(),target],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
            )
        } else {
            return Ok(None);
        }
        .optional()
        .map_err(store_error)?;
    row.map(
        |(
            target,
            terminal,
            generation,
            sequence,
            connection_epoch,
            source,
            occupancy,
            execution,
            top_level,
        )| {
            Ok(SnapshotTargetMatch {
                target: HostTargetId::new(target),
                terminal: terminal.map(TerminalId::new),
                structural_generation: u64::try_from(generation).map_err(|_| {
                    api_error(ErrorCode::StoreCorrupt, "negative snapshot generation")
                })?,
                observation_sequence: u64::try_from(sequence).map_err(|_| {
                    api_error(ErrorCode::StoreCorrupt, "negative snapshot sequence")
                })?,
                connection_epoch: connection_epoch
                    .map(|value| {
                        u64::try_from(value).map_err(|_| {
                            api_error(
                                ErrorCode::StoreCorrupt,
                                "negative snapshot connection epoch",
                            )
                        })
                    })
                    .transpose()?,
                incarnation_source: source
                    .map(|value| {
                        EvidenceKind::from_structural_storage(&value).ok_or_else(|| {
                            api_error(
                                ErrorCode::StoreCorrupt,
                                "invalid snapshot incarnation source",
                            )
                        })
                    })
                    .transpose()?,
                occupancy: match occupancy.as_str() {
                    "empty_shell" => StructuralOccupancy::EmptyShell,
                    "occupied" => StructuralOccupancy::Occupied,
                    "unknown" => StructuralOccupancy::Unknown,
                    _ => {
                        return Err(api_error(
                            ErrorCode::StoreCorrupt,
                            "unknown snapshot occupancy",
                        ));
                    }
                },
                verified_execution: execution.map(ExecutionId::new),
                top_level_occupant: top_level != 0,
            })
        },
    )
    .transpose()
}

fn publication_structural_proof(
    publication: &PublishedSnapshot,
    observed: &SnapshotTargetMatch,
) -> Result<Option<DurableStructuralProof>, ApiError> {
    let (Some(terminal), Some(connection_epoch), Some(source)) = (
        observed.terminal.as_ref(),
        observed.connection_epoch,
        observed.incarnation_source,
    ) else {
        return Ok(None);
    };
    if connection_epoch == 0 || observed.structural_generation == 0 {
        return Ok(None);
    }
    DurableStructuralProof::from_persisted(
        observed.target.clone(),
        terminal.clone(),
        publication.incarnation.clone(),
        source,
        publication.boot.clone(),
        publication.epoch,
        connection_epoch,
        observed.observation_sequence,
        observed.structural_generation,
    )
    .map(Some)
    .map_err(|reason| api_error(ErrorCode::StoreCorrupt, reason))
}

fn update_structural_proof(
    tx: &Transaction<'_>,
    seat: &SeatId,
    proof: &DurableStructuralProof,
) -> Result<(), ApiError> {
    let changed = tx.execute(
        "UPDATE seats SET structural_terminal_id=?1,structural_incarnation=?2,structural_incarnation_kind=?3,structural_host_boot=?4,structural_host_epoch=?5,structural_connection_epoch=?6,structural_observation_sequence=?7 WHERE id=?8 AND target_id=?9 AND target_generation=?10",
        params![proof.terminal().as_str(),proof.incarnation(),proof.source_spelling(),proof.host_boot().as_str(),
            checked_host_number(proof.host_epoch())?,checked_host_number(proof.connection_epoch())?,
            checked_host_number(proof.observation_sequence())?,seat.as_str(),proof.target().as_str(),
            checked_host_number(proof.target_generation())?],
    ).map_err(store_error)?;
    if changed != 1 {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "structural proof target changed within decision",
        ));
    }
    Ok(())
}

fn seat_structural_proof(
    conn: &Connection,
    seat: &SeatId,
) -> Result<Option<DurableStructuralProof>, ApiError> {
    let target: Option<String> = conn
        .query_row(
            "SELECT target_id FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    type StructuralColumns = (
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );
    let structural: StructuralColumns =
        conn.query_row(
            "SELECT target_generation,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        ).map_err(store_error)?;
    let structural_proof = match structural {
        (_, None, None, None, None, None, None, None) => None,
        (
            target_generation,
            Some(terminal),
            Some(incarnation),
            Some(kind),
            Some(boot),
            Some(epoch),
            Some(connection_epoch),
            Some(sequence),
        ) => {
            let target = target.as_ref().ok_or_else(|| {
                api_error(ErrorCode::StoreCorrupt, "structural proof lacks target")
            })?;
            let source = EvidenceKind::from_structural_storage(&kind).ok_or_else(|| {
                api_error(ErrorCode::StoreCorrupt, "invalid structural proof source")
            })?;
            Some(
                DurableStructuralProof::from_persisted(
                    HostTargetId::new(target.clone()),
                    TerminalId::new(terminal),
                    incarnation,
                    source,
                    HostBootId::new(boot),
                    u64::try_from(epoch).map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "negative structural host epoch")
                    })?,
                    u64::try_from(connection_epoch).map_err(|_| {
                        api_error(
                            ErrorCode::StoreCorrupt,
                            "negative structural connection epoch",
                        )
                    })?,
                    u64::try_from(sequence).map_err(|_| {
                        api_error(
                            ErrorCode::StoreCorrupt,
                            "negative structural observation sequence",
                        )
                    })?,
                    u64::try_from(target_generation).map_err(|_| {
                        api_error(
                            ErrorCode::StoreCorrupt,
                            "negative structural target generation",
                        )
                    })?,
                )
                .map_err(|reason| api_error(ErrorCode::StoreCorrupt, reason))?,
            )
        }
        _ => {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "partial structural seat proof",
            ));
        }
    };
    Ok(structural_proof)
}

/// Independent current structure outranks ended native registration history.
/// None permits only an action's explicit legacy/provenance fallback; a
/// present but mismatched proof must never fall back to another identity.
fn structural_continuity_matches(
    conn: &Connection,
    seat: &SeatId,
    publication: &PublishedSnapshot,
    target: Option<&HostTargetId>,
    terminal: Option<&TerminalId>,
) -> Result<Option<bool>, ApiError> {
    Ok(seat_structural_proof(conn, seat)?.map(|proof| {
        Some(proof.target()) == target
            && Some(proof.terminal()) == terminal
            && proof.target_generation() > 0
            && proof.connection_epoch() > 0
            && proof.host_boot() == &publication.boot
            && proof.incarnation() == publication.incarnation
            && proof.host_epoch() <= publication.epoch
            && (proof.host_epoch() < publication.epoch
                || proof.observation_sequence() <= publication.observation_sequence)
    }))
}

// Allowed: one argument per saved seat column.
#[allow(clippy::too_many_arguments)]
fn saved_seat_scalar(
    conn: &Connection,
    ordinal: i64,
    id: String,
    state: String,
    unresolved_reason: Option<String>,
    target: Option<String>,
    generation: i64,
    stage: Option<&SnapshotGenerationId>,
) -> Result<Option<SnapshotSavedSeat>, ApiError> {
    if state == "retired" {
        return Ok(None);
    }
    let structural_proof = seat_structural_proof(conn, &SeatId::new(id.clone()))?;
    let binding: Option<(Option<String>, Option<String>, String, String)> = conn.query_row(
        "SELECT terminal_id,incarnation,execution_id,host_boot FROM occupant_bindings WHERE seat_id=?1 ORDER BY ordinal DESC LIMIT 1",
        [&id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(store_error)?;
    type ActiveBinding = (String, i64, String, Option<String>, String);
    let active_binding: Option<ActiveBinding> = conn
        .query_row(
            "SELECT execution_id,host_epoch,host_boot,incarnation,observation_provenance FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND registered_at IS NOT NULL ORDER BY ordinal DESC LIMIT 1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(store_error)?;
    let bound_epoch =
        match (&active_binding, &structural_proof) {
            // C4: only a binding whose provenance is in the carried set
            // yields a bound epoch; a native binding re-registers instead.
            (Some((_, epoch, boot, incarnation, provenance)), Some(proof))
                if CARRIED_BINDING_PROVENANCES.contains(&provenance.as_str())
                    && boot == proof.host_boot().as_str()
                    && incarnation.as_deref() == Some(proof.incarnation()) =>
            {
                Some(u64::try_from(*epoch).map_err(|_| {
                    api_error(ErrorCode::StoreCorrupt, "negative binding host epoch")
                })?)
            }
            _ => None,
        };
    let active_binding_execution: Option<String> = active_binding.map(|(execution, ..)| execution);
    let prior_published_observation = if state == "unresolved"
        && unresolved_reason.as_deref() == Some("host_invalidation")
    {
        let prior: Option<(String, i64, String, i64, String)> = conn.query_row(
            "SELECT g.id,g.epoch,g.host_boot,s.unresolved_prior_binding_generation,g.incarnation FROM seats s JOIN snapshot_generations g ON g.id=s.unresolved_from_generation_id AND g.status='published' AND g.staged_targets=g.expected_targets WHERE s.id=?1 AND g.instance_id=s.instance_id",
            [&id],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).optional().map_err(store_error)?;
        match prior {
            Some((generation_id, epoch, boot, binding_generation, incarnation)) => {
                let generation_id = SnapshotGenerationId::store_issued(generation_id);
                match snapshot_match(conn, &generation_id, target.as_deref(), None)? {
                    Some(target) => Some(PriorPublishedTarget {
                        generation_id,
                        host_boot: HostBootId::new(boot),
                        host_epoch: u64::try_from(epoch).map_err(|_| {
                            api_error(ErrorCode::StoreCorrupt, "negative prior host epoch")
                        })?,
                        incarnation,
                        prior_binding_generation: u64::try_from(binding_generation).map_err(
                            |_| {
                                api_error(
                                    ErrorCode::StoreCorrupt,
                                    "negative prior binding generation",
                                )
                            },
                        )?,
                        target,
                    }),
                    None => None,
                }
            }
            None => None,
        }
    } else {
        None
    };
    let terminal = structural_proof
        .as_ref()
        .map(|proof| proof.terminal().as_str())
        .or_else(|| {
            binding
                .as_ref()
                .and_then(|value| value.0.as_deref())
                .or_else(|| {
                    prior_published_observation
                        .as_ref()
                        .and_then(|prior| prior.target.terminal.as_ref().map(TerminalId::as_str))
                })
        });
    let observed_match = match stage {
        Some(stage) => snapshot_match(conn, stage, target.as_deref(), terminal)?,
        None => None,
    };
    Ok(Some(SnapshotSavedSeat {
        ordinal: u64::try_from(ordinal)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative seat ordinal"))?,
        seat: SeatId::new(id),
        state: match state.as_str() {
            "resolved" => SeatState::Resolved,
            "unresolved" => SeatState::Unresolved,
            _ => {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "unknown saved-seat state",
                ));
            }
        },
        unresolved_reason: match unresolved_reason.as_deref() {
            None => None,
            Some("host_invalidation") => Some(UnresolvedReason::HostInvalidation),
            Some("other") => Some(UnresolvedReason::Other),
            Some(_) => {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "unknown unresolved reason",
                ));
            }
        },
        prior_published_observation: prior_published_observation.clone(),
        structural_proof: structural_proof.clone(),
        target: target.map(HostTargetId::new),
        terminal: terminal.map(TerminalId::new),
        binding_generation: u64::try_from(generation)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative seat generation"))?,
        binding_execution: binding
            .as_ref()
            .map(|value| ExecutionId::new(value.2.clone())),
        active_binding_execution: active_binding_execution.map(ExecutionId::new),
        bound_epoch,
        bound_boot: structural_proof
            .as_ref()
            .map(|proof| proof.host_boot().clone())
            .or_else(|| {
                binding
                    .as_ref()
                    .map(|value| HostBootId::new(value.3.clone()))
                    .or_else(|| {
                        prior_published_observation
                            .as_ref()
                            .map(|prior| prior.host_boot.clone())
                    })
            }),
        bound_incarnation: structural_proof
            .as_ref()
            .map(|proof| proof.incarnation().to_owned())
            .or_else(|| {
                binding
                    .as_ref()
                    .and_then(|value| value.1.clone())
                    .or_else(|| {
                        prior_published_observation
                            .as_ref()
                            .map(|prior| prior.incarnation.clone())
                    })
            }),
        latest_binding_evidence: binding.as_ref().and_then(|value| {
            match (value.0.as_deref(), value.1.as_deref()) {
                (Some(terminal), Some(incarnation))
                    if !terminal.is_empty() && !incarnation.is_empty() =>
                {
                    Some(BindingEvidence {
                        terminal: TerminalId::new(terminal),
                        host_boot: HostBootId::new(value.3.clone()),
                        incarnation: incarnation.to_owned(),
                    })
                }
                _ => None,
            }
        }),
        observed_match,
    }))
}

pub fn saved_seats_page(
    context: &StoreContext,
    conn: &Connection,
    published: &SnapshotGenerationId,
    after_ordinal: u64,
    high_water_ordinal: Option<u64>,
    limit: u8,
    budget: &CallBudget,
) -> Result<SnapshotSeatPage, ApiError> {
    snapshot_budget(context, budget)?;
    if limit == 0 || limit > 16 {
        return Err(api_error(
            ErrorCode::InvalidBudget,
            "saved-seat page limit must be 1..16",
        ));
    }
    let read_tx = conn.unchecked_transaction().map_err(store_error)?;
    let conn: &Connection = &read_tx;
    let after = checked_host_number(after_ordinal)?;
    let publication = active_publication(conn, published)?;
    let high_water = match high_water_ordinal {
        Some(value) => checked_host_number(value)?,
        None => conn
            .query_row(
                "SELECT COALESCE(MAX(ordinal),0) FROM seats WHERE instance_id=?1",
                [&publication.instance],
                |r| r.get(0),
            )
            .map_err(store_error)?,
    };
    let mut stmt = conn
        .prepare(
            "SELECT ordinal,id,state,unresolved_reason,target_id,generation FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND state!='retired' AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4",
        )
        .map_err(store_error)?;
    let mut rows = stmt
        .query(params![
            publication.instance,
            after,
            high_water,
            i64::from(limit)
        ])
        .map_err(store_error)?;
    let mut seats = Vec::new();
    let mut last = after;
    let mut visited = 0_u8;
    while let Some(row) = rows.next().map_err(store_error)? {
        snapshot_budget(context, budget)?;
        let (ordinal, id, state, unresolved_reason, target, generation): (
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
        ) = (
            row.get(0).map_err(store_error)?,
            row.get(1).map_err(store_error)?,
            row.get(2).map_err(store_error)?,
            row.get(3).map_err(store_error)?,
            row.get(4).map_err(store_error)?,
            row.get(5).map_err(store_error)?,
        );
        visited += 1;
        last = ordinal;
        if let Some(saved) = saved_seat_scalar(
            conn,
            ordinal,
            id,
            state,
            unresolved_reason,
            target,
            generation,
            Some(published),
        )? {
            seats.push(saved);
        }
    }
    let has_more: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND state!='retired' AND ordinal>?2 AND ordinal<=?3)",
            params![publication.instance,last,high_water],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    Ok(SnapshotSeatPage {
        publication,
        high_water_ordinal: high_water as u64,
        after_ordinal: last as u64,
        seats,
        visited,
        has_more,
    })
}

pub fn saved_seats_page_for_invalidation(
    context: &StoreContext,
    conn: &Connection,
    fence: &HostInvalidationFence,
    after_ordinal: u64,
    high_water_ordinal: Option<u64>,
    limit: u8,
    budget: &CallBudget,
) -> Result<InvalidationSeatPage, ApiError> {
    snapshot_budget(context, budget)?;
    if limit == 0 || limit > 16 {
        return Err(api_error(
            ErrorCode::InvalidBudget,
            "saved-seat page limit must be 1..16",
        ));
    }
    let read_tx = conn.unchecked_transaction().map_err(store_error)?;
    let conn: &Connection = &read_tx;
    let after = checked_host_number(after_ordinal)?;
    let current: Option<(i64, i64)> = conn
        .query_row(
            "SELECT invalidation_revision,observation_decided_sequence FROM host_instances WHERE id=?1",
            [fence.instance()],
            |r| Ok((r.get(0)?,r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    if current
        != Some((
            checked_host_number(fence.invalidation_revision())?,
            checked_host_number(fence.admission_sequence())?,
        ))
    {
        return Err(api_error(
            ErrorCode::CursorStale,
            "host invalidation changed",
        ));
    }
    let high_water = match high_water_ordinal {
        Some(value) => checked_host_number(value)?,
        None => conn
            .query_row(
                "SELECT COALESCE(MAX(ordinal),0) FROM seats WHERE instance_id=?1",
                [fence.instance()],
                |r| r.get(0),
            )
            .map_err(store_error)?,
    };
    let mut stmt = conn
        .prepare(
            "SELECT ordinal,id,state,unresolved_reason,target_id,generation FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND state!='retired' AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4",
        )
        .map_err(store_error)?;
    let mut rows = stmt
        .query(params![
            fence.instance(),
            after,
            high_water,
            i64::from(limit)
        ])
        .map_err(store_error)?;
    let mut seats = Vec::new();
    let mut last = after;
    let mut visited = 0_u8;
    while let Some(row) = rows.next().map_err(store_error)? {
        snapshot_budget(context, budget)?;
        let (ordinal, id, state, unresolved_reason, target, generation): (
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
        ) = (
            row.get(0).map_err(store_error)?,
            row.get(1).map_err(store_error)?,
            row.get(2).map_err(store_error)?,
            row.get(3).map_err(store_error)?,
            row.get(4).map_err(store_error)?,
            row.get(5).map_err(store_error)?,
        );
        visited += 1;
        last = ordinal;
        if let Some(saved) = saved_seat_scalar(
            conn,
            ordinal,
            id,
            state,
            unresolved_reason,
            target,
            generation,
            None,
        )? {
            seats.push(saved);
        }
    }
    let has_more: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND state!='retired' AND ordinal>?2 AND ordinal<=?3)",
            params![fence.instance(),last,high_water],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    Ok(InvalidationSeatPage {
        fence: fence.clone(),
        high_water_ordinal: high_water as u64,
        after_ordinal: last as u64,
        seats,
        visited,
        has_more,
    })
}

pub fn discard_snapshot_stage(
    context: &StoreContext,
    conn: &mut Connection,
    stage: &SnapshotGenerationId,
    admission: DurableWorkAdmission,
    budget: &CallBudget,
) -> Result<SnapshotCleanupProgress, ApiError> {
    snapshot_budget(context, budget)?;
    if admission.max_units == 0 || admission.max_units > 16 {
        return Err(api_error(
            ErrorCode::InvalidBudget,
            "snapshot cleanup admission must be 1..16",
        ));
    }
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let started = context.clock().monotonic_now();
    let (instance, _, _, _, _, _, _, status) = load_snapshot_stage(&tx, stage)?;
    if status == "published" {
        let retained: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM host_instances WHERE id=?1 AND (active_snapshot_id=?2 OR recovery_baseline_generation_id=?2)) OR EXISTS(SELECT 1 FROM seats WHERE unresolved_from_generation_id=?2)",
                params![instance, stage.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if retained {
            return Err(api_error(
                ErrorCode::Conflict,
                "published snapshot is still retained",
            ));
        }
    }
    if status != "discarded" {
        tx.execute(
            "UPDATE snapshot_generations SET status='discarded' WHERE id=?1",
            [stage.as_str()],
        )
        .map_err(store_error)?;
    }
    let ordinals: Vec<i64> = {
        let mut stmt = tx
            .prepare(
                "SELECT ordinal FROM snapshot_targets WHERE generation_id=?1 ORDER BY ordinal LIMIT ?2",
            )
            .map_err(store_error)?;
        stmt.query_map(
            params![stage.as_str(), i64::from(admission.max_units)],
            |r| r.get(0),
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?
    };
    // As in staging: a spent quantum ends the turn with its committed prefix
    // (at least one row); `complete` stays false and cleanup resumes.
    let mut deleted = 0usize;
    for ordinal in &ordinals {
        snapshot_budget(context, budget)?;
        if deleted > 0 && context.clock().monotonic_now().0.saturating_sub(started.0) >= 5 {
            break;
        }
        tx.execute("DELETE FROM snapshot_targets WHERE ordinal=?1", [ordinal])
            .map_err(store_error)?;
        deleted += 1;
    }
    let remaining: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM snapshot_targets WHERE generation_id=?1)",
            [stage.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    tx.commit().map_err(store_error)?;
    Ok(SnapshotCleanupProgress {
        stage: stage.clone(),
        visited: deleted as u8,
        complete: !remaining,
    })
}

fn saved_seat_cas(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    expected_generation: u64,
    expected_target: Option<&HostTargetId>,
    expected_terminal: Option<&TerminalId>,
    allow_host_invalidated: bool,
) -> Result<bool, ApiError> {
    let generation = checked_host_number(expected_generation)?;
    type SavedColumns = Option<(String, Option<String>, i64, Option<String>, Option<String>)>;
    let saved: SavedColumns = tx
        .query_row(
            "SELECT state,unresolved_reason,generation,target_id,structural_terminal_id FROM seats WHERE id=?1 AND instance_id=?2",
            params![seat.as_str(), instance],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((state, reason, current_generation, target, structural_terminal)) = saved else {
        return Ok(false);
    };
    let invalidated = allow_host_invalidated
        && state == "unresolved"
        && reason.as_deref() == Some("host_invalidation");
    if state != "resolved" && !invalidated
        || current_generation != generation
        || target.as_deref() != expected_target.map(|id| id.as_str())
    {
        return Ok(false);
    }
    let binding_query = if invalidated {
        "SELECT terminal_id FROM occupant_bindings WHERE seat_id=?1 ORDER BY ordinal DESC LIMIT 1"
    } else {
        "SELECT terminal_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL"
    };
    let mut terminal: Option<String> = tx
        .query_row(binding_query, [seat.as_str()], |r| r.get(0))
        .optional()
        .map_err(store_error)?
        .flatten();
    if structural_terminal.is_some() {
        terminal = structural_terminal;
    }
    if invalidated && terminal.is_none() {
        terminal = tx.query_row(
            "SELECT t.terminal_id FROM seats s JOIN snapshot_generations g ON g.id=s.unresolved_from_generation_id AND g.status='published' AND g.staged_targets=g.expected_targets JOIN snapshot_targets t ON t.generation_id=g.id AND t.target_id=s.target_id WHERE s.id=?1 AND s.instance_id=?2 AND s.unresolved_prior_binding_generation=?3",
            params![seat.as_str(),instance,generation-1],
            |r| r.get(0),
        ).optional().map_err(store_error)?.flatten();
    }
    Ok(terminal.as_deref() == expected_terminal.map(|id| id.as_str()))
}

/// TRUST-POLICY C2: instance-wide restore-hold release when nothing is left
/// to protect. Predicate: reconciliation of the current recovery boot/epoch
/// finished (persisted marker) and no unresolved nonretired seat remains.
/// Clears `baseline_hold_unclaimed` and releases every open `recovery_holds`
/// row of the instance in the caller's transaction. Returns whether it lifted.
pub(crate) fn lift_baseline_hold_if_clear(
    tx: &Transaction<'_>,
    instance: &str,
    at: UtcMillis,
) -> Result<bool, ApiError> {
    type Marker = Option<(
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<i64>,
        i64,
    )>;
    let row: Marker = tx
        .query_row(
            "SELECT recovery_boot,recovery_epoch,reconciled_boot,reconciled_epoch,baseline_hold_unclaimed FROM host_instances WHERE id=?1",
            [instance],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((recovery_boot, recovery_epoch, reconciled_boot, reconciled_epoch, flag)) = row else {
        return Ok(false);
    };
    if recovery_boot.is_none()
        || recovery_boot != reconciled_boot
        || recovery_epoch != reconciled_epoch
    {
        return Ok(false);
    }
    let unresolved: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE instance_id=?1 AND state='unresolved')",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let open_holds: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE instance_id=?1 AND released_at IS NULL)",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if unresolved || (flag == 0 && !open_holds) {
        return Ok(false);
    }
    tx.execute(
        "UPDATE host_instances SET baseline_hold_unclaimed=0 WHERE id=?1",
        [instance],
    )
    .map_err(store_error)?;
    tx.execute(
        "UPDATE recovery_holds SET released_at=?1 WHERE instance_id=?2 AND released_at IS NULL",
        params![at.0, instance],
    )
    .map_err(store_error)?;
    schema::bump_lifecycle_revision(tx, instance)?;
    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
    Ok(true)
}

/// Persist that the saved-seat pass of `published` finished with no refused
/// transition, then run the hold lift in the same transaction. Writes only
/// while `published` is still the active publication of the instance's current
/// recovery boot/epoch; returns whether the marker was written.
pub fn record_reconciliation_pass(
    context: &StoreContext,
    conn: &mut Connection,
    published: &PublishedSnapshot,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    snapshot_budget(context, budget)?;
    let epoch = checked_host_number(published.epoch)?;
    context.execute_budgeted_decision(
        conn,
        budget,
        |tx| {
            let current = match active_publication(tx, &published.id) {
                Ok(current) => current,
                Err(error) if error.code == ErrorCode::CursorStale => return Ok(false),
                Err(error) => return Err(error),
            };
            if current != *published {
                return Ok(false);
            }
            let recovery: Option<(Option<String>, Option<i64>)> = tx
                .query_row(
                    "SELECT recovery_boot,recovery_epoch FROM host_instances WHERE id=?1",
                    [published.instance.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            Ok(recovery == Some((Some(published.boot.as_str().to_owned()), Some(epoch))))
        },
        |tx, at, current| {
            if !current {
                return Ok(false);
            }
            tx.execute(
                "UPDATE host_instances SET reconciled_boot=?1,reconciled_epoch=?2 WHERE id=?3",
                params![published.boot.as_str(), epoch, published.instance],
            )
            .map_err(store_error)?;
            lift_baseline_hold_if_clear(tx, &published.instance, at.utc)?;
            Ok(true)
        },
    )
}

fn mark_seat_unresolved(
    tx: &Transaction<'_>,
    at: DecisionInstant,
    instance: &str,
    seat: &SeatId,
    generation: u64,
    reason: UnresolvedReason,
) -> Result<ReconciliationOutcome, ApiError> {
    let generation = checked_host_number(generation)?;
    if generation == i64::MAX {
        return Err(api_error(
            ErrorCode::SequenceExhausted,
            "seat binding generation exhausted",
        ));
    }
    let prior_generation: Option<String> = if reason == UnresolvedReason::HostInvalidation {
        tx.query_row(
        "SELECT g.id FROM seats s JOIN host_instances h ON h.id=s.instance_id JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.status='published' AND g.staged_targets=g.expected_targets JOIN snapshot_targets t ON t.generation_id=g.id AND t.target_id=s.target_id WHERE s.id=?1 AND s.instance_id=?2 AND s.state='resolved' AND s.generation=?3 AND t.terminal_id IS NOT NULL",
        params![seat.as_str(), instance, generation],
        |r| r.get(0),
    ).optional().map_err(store_error)?
    } else {
        None
    };
    let reason_sql = match reason {
        UnresolvedReason::HostInvalidation => "host_invalidation",
        UnresolvedReason::Other => "other",
    };
    let changed = tx
        .execute(
            "UPDATE seats SET state='unresolved',unresolved_reason=?4,unresolved_from_generation_id=?5,unresolved_prior_binding_generation=CASE WHEN ?5 IS NULL THEN NULL ELSE generation END,generation=generation+1 WHERE id=?1 AND instance_id=?2 AND generation=?3 AND state='resolved'",
            params![seat.as_str(),instance,generation,reason_sql,prior_generation],
        )
        .map_err(store_error)?;
    if changed != 1 {
        return Ok(ReconciliationOutcome::Stale);
    }
    tx.execute(
        "UPDATE occupant_bindings SET ended_at=?1 WHERE seat_id=?2 AND ended_at IS NULL",
        params![at.utc.0, seat.as_str()],
    )
    .map_err(store_error)?;
    catch_up::supersede_stale(tx, seat, at.utc)?;
    tx.execute(
        "DELETE FROM warning_offer WHERE seat_id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    tx.execute(
        "UPDATE wake_work SET reservation_id=NULL,reservation_boot=NULL,binding_generation=NULL WHERE seat_id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    schema::ensure_unavailability_episode(tx, seat)?;
    tx.execute(
        "UPDATE host_instances SET baseline_hold_unclaimed=1 WHERE id=?1 AND recovery_baseline_generation_id IS NOT NULL",
        [instance],
    )
    .map_err(store_error)?;
    schema::bump_lifecycle_revision(tx, instance)?;
    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
    Ok(ReconciliationOutcome::Applied)
}

pub fn mark_unresolved_from_invalidation(
    context: &StoreContext,
    conn: &mut Connection,
    transition: GuardedInvalidationTransition,
    budget: &CallBudget,
) -> Result<ReconciliationOutcome, ApiError> {
    snapshot_budget(context, budget)?;
    checked_host_number(transition.fence.invalidation_revision)?;
    context.execute_budgeted_decision(
        conn, budget,
        |tx| {
            let instance = transition.fence.instance();
            let current: Option<(i64, i64, Option<String>)> = tx
                .query_row(
                    "SELECT invalidation_revision,observation_decided_sequence,active_snapshot_id FROM host_instances WHERE id=?1",
                    [instance],
                    |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
                )
                .optional()
                .map_err(store_error)?;
            let Some((revision, decided, active)) = current else {
                return Ok(false);
            };
            if revision != transition.fence.invalidation_revision as i64
                || decided != transition.fence.admission_sequence() as i64
            {
                return Ok(false);
            }
            if let Some(active) = active {
                let published_revision: Option<i64> = tx
                    .query_row(
                        "SELECT published_invalidation_revision FROM snapshot_generations WHERE id=?1",
                        [&active],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(store_error)?
                    .flatten();
                if published_revision.is_some_and(|published| published >= revision) {
                    return Ok(false);
                }
            }
            saved_seat_cas(
                tx,
                instance,
                &transition.seat,
                transition.expected_binding_generation,
                transition.expected_target.as_ref(),
                transition.expected_terminal.as_ref(),
                false,
            )
        },
        |tx, at, valid| {
            if !valid {
                return Ok(ReconciliationOutcome::Stale);
            }
            mark_seat_unresolved(
                tx,
                at,
                transition.fence.instance(),
                &transition.seat,
                transition.expected_binding_generation,
                UnresolvedReason::HostInvalidation,
            )
        },
    )
}

/// C4: carry the open registered binding forward to the publication's host
/// epoch in place. Harness, session, execution, generation and provenance stay.
/// The single definition of carrying: returns whether a binding moved. When
/// one did it writes the same availability anchor and receipt-timer job a
/// fresh registration writes, closes the seat's open unavailability marker and
/// re-points pending wake work. Moves nothing for a native binding.
#[allow(clippy::too_many_arguments)]
fn carry_binding_forward(
    tx: &Transaction<'_>,
    instance: &str,
    at: UtcMillis,
    seat: &SeatId,
    generation: u64,
    publication: &PublishedSnapshot,
    target: &str,
    target_generation: i64,
) -> Result<bool, ApiError> {
    let changed = tx.execute(
        "UPDATE occupant_bindings SET host_epoch=?1,target_generation=?2 WHERE seat_id=?3 AND generation=?4 AND ended_at IS NULL AND registered_at IS NOT NULL AND host_boot=?5 AND target_id=?6 AND incarnation=?7 AND observation_provenance IN (?8,?9) AND (host_epoch!=?1 OR target_generation!=?2)",
        params![
            checked_host_number(publication.epoch)?,
            target_generation,
            seat.as_str(),
            checked_host_number(generation)?,
            publication.boot.as_str(),
            target,
            publication.incarnation,
            CARRIED_BINDING_PROVENANCES[0],
            CARRIED_BINDING_PROVENANCES[1]
        ],
    ).map_err(store_error)?;
    if changed != 1 {
        return Ok(false);
    }
    let (binding_generation, provenance): (i64, String) = tx
        .query_row(
            "SELECT generation,observation_provenance FROM occupant_bindings WHERE seat_id=?1 AND generation=?2 AND ended_at IS NULL",
            params![seat.as_str(), checked_host_number(generation)?],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    let seq = schema::next_decision_seq(tx, instance)?;
    anchor_seat_availability(tx, seat, seq, at, binding_generation, &provenance)?;
    schema::clear_open_unavailability_for_seat(tx, seat.as_str(), at)?;
    tx.execute(
        "UPDATE seats SET unavailability_open=0 WHERE id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    tx.execute(
        "UPDATE wake_work SET binding_generation=?1 WHERE seat_id=?2 AND reservation_id IS NULL",
        params![binding_generation, seat.as_str()],
    )
    .map_err(store_error)?;
    Ok(true)
}

/// A binding just became available: its availability anchor and the
/// receipt-timer job for every recipient row already staged for the seat
/// (same rows as a fresh registration writes).
fn anchor_seat_availability(
    tx: &Transaction<'_>,
    seat: &SeatId,
    seq: u64,
    at: UtcMillis,
    binding_generation: i64,
    provenance: &str,
) -> Result<(), ApiError> {
    tx.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES (?1,?2,?3,?4,?5)",
        params![seat.as_str(), seq as i64, at.0, binding_generation, provenance]).map_err(store_error)?;
    let anchor = tx.last_insert_rowid();
    let high_water: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(ordinal),0) FROM prepared_recipients WHERE seat_id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'receipt_timer_materialization',?2,?3)",
        params![format!("receipt-timer:{anchor}"), anchor.to_string(), high_water]).map_err(store_error)?;
    Ok(())
}

pub fn apply_reconciliation_transition(
    context: &StoreContext,
    conn: &mut Connection,
    transition: GuardedSeatTransition,
    budget: &CallBudget,
) -> Result<ReconciliationOutcome, ApiError> {
    snapshot_budget(context, budget)?;
    checked_host_number(transition.expected_binding_generation)?;
    context.execute_budgeted_decision(
        conn, budget,
        |tx| {
            let publication = match active_publication(tx, &transition.publication.id) {
                Ok(publication) => publication,
                Err(error) if error.code == ErrorCode::CursorStale => return Ok(None),
                Err(error) => return Err(error),
            };
            if publication != transition.publication {
                return Ok(None);
            }
            // The plan keeps an already-unresolved seat unresolved when this
            // publication cannot reconfirm it (for example its agent exited
            // during the outage). That is a no-op, not a stale plan: treating
            // it as Stale aborted the whole page with CursorStale on every
            // turn, so one unrecoverable seat blocked recovery of every later
            // seat. The seat keeps its reason, so later evidence may still
            // reconfirm a host-invalidated seat.
            if matches!(transition.action, ReconciliationAction::MarkUnresolved) {
                let already_unresolved: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2 AND state='unresolved' AND generation=?3 AND target_id IS ?4)",
                    params![transition.seat.as_str(),publication.instance,checked_host_number(transition.expected_binding_generation)?,transition.expected_target.as_ref().map(HostTargetId::as_str)],
                    |r| r.get(0),
                ).map_err(store_error)?;
                if already_unresolved {
                    return Ok(Some((0, None, true)));
                }
            }
            if !saved_seat_cas(
                    tx,
                    &publication.instance,
                    &transition.seat,
                    transition.expected_binding_generation,
                    transition.expected_target.as_ref(),
                    transition.expected_terminal.as_ref(),
                    matches!(transition.action, ReconciliationAction::ReconfirmStructure { .. } | ReconciliationAction::BeginRetirement { .. }),
                )?
            {
                return Ok(None);
            }
            let generation = match &transition.action {
                ReconciliationAction::MarkUnresolved => 0,
                ReconciliationAction::ReconfirmStructure { target, terminal }
                | ReconciliationAction::CarryForward { target, terminal } => {
                    if matches!(transition.action, ReconciliationAction::CarryForward { .. }) {
                        let resolved: bool = tx.query_row(
                            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND state='resolved')",
                            [transition.seat.as_str()], |r| r.get(0),
                        ).map_err(store_error)?;
                        if !resolved { return Ok(None); }
                    }
                    // Same terminal, same verified boot and incarnation as the
                    // evidence on the seat's latest binding; occupancy and
                    // execution are not consulted (the production adapter
                    // reports both Unknown).
                    if transition.expected_terminal.as_ref() != Some(terminal) {
                        return Ok(None);
                    }
                    let Some(observed) = snapshot_match(tx, &publication.id, Some(target.as_str()), Some(terminal.as_str()))? else {
                        return Ok(None);
                    };
                    if observed.target != *target
                        || observed.terminal.as_ref() != Some(terminal)
                        || observed.structural_generation == 0
                        || observed.observation_sequence == 0
                        || observed.observation_sequence > publication.observation_sequence
                        || publication_structural_proof(&publication, &observed)?.is_none()
                    {
                        return Ok(None);
                    }
                    let bound: Option<(Option<String>, String, Option<String>)> = tx.query_row(
                        "SELECT terminal_id,host_boot,incarnation FROM occupant_bindings WHERE seat_id=?1 ORDER BY ordinal DESC LIMIT 1",
                        [transition.seat.as_str()],
                        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
                    ).optional().map_err(store_error)?;
                    let evidence_matches = match bound {
                        Some((bound_terminal, boot, incarnation)) => {
                            bound_terminal.as_deref() == Some(terminal.as_str())
                                && boot == publication.boot.as_str()
                                && incarnation.as_deref() == Some(publication.incarnation.as_str())
                        }
                        // Resolved but never registered: no binding row at
                        // all, so the seat's own verified structural proof
                        // is the evidence, under the same terminal, boot and
                        // incarnation rule (wave-2 fix2 (b)).
                        None => structural_continuity_matches(
                            tx,
                            &transition.seat,
                            &publication,
                            transition.expected_target.as_ref(),
                            Some(terminal),
                        )? == Some(true),
                    };
                    if !evidence_matches {
                        return Ok(None);
                    }
                    let owned: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM seats WHERE instance_id=?1 AND target_id=?2 AND state='resolved' AND id!=?3)",
                        params![publication.instance,target.as_str(),transition.seat.as_str()],
                        |r| r.get(0),
                    ).map_err(store_error)?;
                    if owned { return Ok(None); }
                    checked_host_number(observed.structural_generation)?
                }
                ReconciliationAction::Move { target, terminal } => {
                    let observed = snapshot_match(
                        tx,
                        &publication.id,
                        Some(target.as_str()),
                        Some(terminal.as_str()),
                    )?;
                    let Some(observed) = observed else {
                        return Ok(None);
                    };
                    if observed.target != *target
                        || observed.terminal.as_ref() != Some(terminal)
                        || observed.structural_generation == 0
                        || observed.observation_sequence == 0
                        || observed.observation_sequence > publication.observation_sequence
                    {
                        return Ok(None);
                    }
                    if publication_structural_proof(&publication, &observed)?.is_none() {
                        return Ok(None);
                    }
                    let owned: bool = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM seats WHERE instance_id=?1 AND target_id=?2 AND state='resolved' AND id!=?3)",
                            params![publication.instance,target.as_str(),transition.seat.as_str()],
                            |r| r.get(0),
                        )
                        .map_err(store_error)?;
                    if owned {
                        return Ok(None);
                    }
                    let bound: Option<(String, Option<String>, String)> = tx
                        .query_row(
                            "SELECT host_boot,incarnation,execution_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
                            [transition.seat.as_str()],
                            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
                        )
                        .optional()
                        .map_err(store_error)?;
                    let structural_matches = structural_continuity_matches(
                        tx, &transition.seat, &publication,
                        transition.expected_target.as_ref(), Some(terminal),
                    )?;
                    if structural_matches == Some(false)
                        || structural_matches.is_none() && !bound.as_ref().is_some_and(|(boot, incarnation, _)| {
                            boot == publication.boot.as_str()
                                && incarnation.as_deref() == Some(&publication.incarnation)
                        })
                    { return Ok(None); }
                    if let Some((_, _, bound_execution)) = bound.as_ref() {
                        match observed.verified_execution.as_ref() {
                            // A verified execution must be the bound one: a
                            // different one is a replacement, never a move.
                            Some(execution) => {
                                if observed.occupancy != StructuralOccupancy::Occupied
                                    || !observed.top_level_occupant
                                    || execution.as_str() != bound_execution
                                {
                                    return Ok(None);
                                }
                            }
                            // Unknown execution never unseats a binding (D2):
                            // the same terminal in the same verified server
                            // incarnation (checked above) carries the seat,
                            // its generation and its binding structurally to
                            // the new address. A registered binding whose
                            // occupant the snapshot shows absent is not carried.
                            None => {
                                let (bound_terminal, registered): (Option<String>, bool) = tx.query_row(
                                    "SELECT terminal_id,registered_at IS NOT NULL FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
                                    [transition.seat.as_str()],
                                    |r| Ok((r.get(0)?, r.get(1)?)),
                                ).map_err(store_error)?;
                                let same_terminal = structural_matches == Some(true)
                                    || bound_terminal.as_deref() == Some(terminal.as_str());
                                if !same_terminal || registered && observed.shows_occupant_absent() {
                                    return Ok(None);
                                }
                            }
                        }
                    } else if observed.verified_execution.is_some() || observed.top_level_occupant {
                        // Loss ended registration, not the independent pane identity.
                        // Reappearance of that execution may move structure only.
                        let historical: Option<String> = tx.query_row(
                            "SELECT execution_id FROM occupant_bindings WHERE seat_id=?1 AND generation=?2 AND ended_at IS NOT NULL ORDER BY ordinal DESC LIMIT 1",
                            params![transition.seat.as_str(),checked_host_number(transition.expected_binding_generation)?],
                            |r| r.get(0),
                        ).optional().map_err(store_error)?;
                        if structural_matches != Some(true)
                            || historical.is_none()
                            || observed.verified_execution.is_none()
                            || observed.occupancy != StructuralOccupancy::Occupied
                            || !observed.top_level_occupant
                            || observed.verified_execution.as_ref().map(ExecutionId::as_str) != historical.as_deref()
                        { return Ok(None); }
                    }
                    checked_host_number(observed.structural_generation)?
                }
                ReconciliationAction::BeginRetirement { absent_target } => {
                    if transition.expected_target.as_ref() != Some(absent_target) {
                        return Ok(None);
                    }
                    let state: String = tx.query_row("SELECT state FROM seats WHERE id=?1", [transition.seat.as_str()], |r| r.get(0)).map_err(store_error)?;
                    let bound = if state == "resolved" {
                        match structural_continuity_matches(
                            tx, &transition.seat, &publication,
                            transition.expected_target.as_ref(), transition.expected_terminal.as_ref(),
                        )? {
                            Some(true) => Some((publication.boot.as_str().to_owned(), Some(publication.incarnation.clone()))),
                            Some(false) => return Ok(None),
                            None => tx.query_row(
                                "SELECT host_boot,incarnation FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
                                [transition.seat.as_str()], |r| Ok((r.get(0)?,r.get(1)?)),
                            ).optional().map_err(store_error)?,
                        }
                    } else {
                        // Unresolved recovery remains restricted by saved_seat_cas
                        // to host invalidation and its retained published provenance.
                        let historical: Option<(String, Option<String>)> = tx.query_row(
                            "SELECT host_boot,incarnation FROM occupant_bindings WHERE seat_id=?1 ORDER BY ordinal DESC LIMIT 1",
                            [transition.seat.as_str()], |r| Ok((r.get(0)?,r.get(1)?)),
                        ).optional().map_err(store_error)?;
                        match historical {
                            Some(bound) => Some(bound),
                            None => tx.query_row(
                                "SELECT g.host_boot,g.incarnation FROM seats s JOIN snapshot_generations g ON g.id=s.unresolved_from_generation_id AND g.status='published' AND g.staged_targets=g.expected_targets JOIN snapshot_targets t ON t.generation_id=g.id AND t.target_id=s.target_id WHERE s.id=?1 AND s.instance_id=?2 AND s.state='unresolved' AND s.unresolved_reason='host_invalidation' AND s.unresolved_prior_binding_generation=?3 AND t.terminal_id=?4",
                                params![transition.seat.as_str(),publication.instance,checked_host_number(transition.expected_binding_generation)?.saturating_sub(1),transition.expected_terminal.as_ref().map(TerminalId::as_str)],
                                |r| Ok((r.get(0)?,Some(r.get(1)?))),
                            ).optional().map_err(store_error)?,
                        }
                    };
                    let Some((bound_boot, bound_incarnation)) = bound else { return Ok(None); };
                    let terminal_present = match transition.expected_terminal.as_ref() {
                        Some(terminal) => snapshot_match(
                            tx,
                            &publication.id,
                            None,
                            Some(terminal.as_str()),
                        )?
                        .is_some(),
                        None => false,
                    };
                    if bound_boot != publication.boot.as_str()
                        || bound_incarnation.as_deref() != Some(&publication.incarnation)
                        || snapshot_match(tx, &publication.id, Some(absent_target.as_str()), None)?.is_some()
                        || terminal_present
                    {
                        return Ok(None);
                    }
                    let target_generation: i64 = tx
                        .query_row(
                            "SELECT target_generation FROM seats WHERE id=?1",
                            [transition.seat.as_str()],
                            |r| r.get(0),
                        )
                        .map_err(store_error)?;
                    if target_generation <= 0 || transition.expected_binding_generation == i64::MAX as u64 {
                        return Ok(None);
                    }
                    target_generation
                }
            };
            let proof = match &transition.action {
                ReconciliationAction::ReconfirmStructure { target, terminal }
                | ReconciliationAction::CarryForward { target, terminal }
                | ReconciliationAction::Move { target, terminal } => {
                    let observed = snapshot_match(tx, &publication.id, Some(target.as_str()), Some(terminal.as_str()))?
                        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "validated snapshot target disappeared"))?;
                    publication_structural_proof(&publication, &observed)?
                }
                _ => None,
            };
            Ok(Some((generation, proof, false)))
        },
        |tx, at, prepared| {
            let Some((target_generation, proof, already_unresolved)) = prepared else {
                return Ok(ReconciliationOutcome::Stale);
            };
            if already_unresolved {
                return Ok(ReconciliationOutcome::Unchanged);
            }
            let instance = &transition.publication.instance;
            let seat = &transition.seat;
            let outcome = match &transition.action {
                ReconciliationAction::MarkUnresolved => mark_seat_unresolved(
                    tx,
                    at,
                    instance,
                    seat,
                    transition.expected_binding_generation,
                    UnresolvedReason::Other,
                )?,
                ReconciliationAction::ReconfirmStructure { target, .. } => {
                    let changed = tx.execute(
                        "UPDATE seats SET state='resolved',unresolved_reason=NULL,unresolved_from_generation_id=NULL,unresolved_prior_binding_generation=NULL,target_id=?1,target_generation=?2 WHERE id=?3 AND instance_id=?4 AND generation=?5 AND state='unresolved' AND unresolved_reason='host_invalidation'",
                        params![target.as_str(),target_generation,seat.as_str(),instance,checked_host_number(transition.expected_binding_generation)?],
                    ).map_err(store_error)?;
                    if changed != 1 { return Ok(ReconciliationOutcome::Stale); }
                    // Nothing to carry: the invalidation that unresolved the
                    // seat ended its bindings and bumped its generation, and no
                    // check-in registers while it is unresolved (ht-0b8).
                    update_structural_proof(tx, seat, proof.as_ref().expect("validated structural proof"))?;
                    schema::bump_lifecycle_revision(tx, instance)?;
                    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
                    lift_baseline_hold_if_clear(tx, instance, at.utc)?;
                    ReconciliationOutcome::Applied
                }
                ReconciliationAction::CarryForward { target, .. } => {
                    let generation = checked_host_number(transition.expected_binding_generation)?;
                    let resolved: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND generation=?2 AND state='resolved')",
                        params![seat.as_str(),generation], |r| r.get(0),
                    ).map_err(store_error)?;
                    if !resolved { return Ok(ReconciliationOutcome::Stale); }
                    // C4: a carry that moves no binding changes nothing.
                    if !carry_binding_forward(tx, instance, at.utc, seat, transition.expected_binding_generation, &transition.publication, target.as_str(), target_generation)? {
                        return Ok(ReconciliationOutcome::Unchanged);
                    }
                    tx.execute(
                        "UPDATE seats SET target_generation=?1 WHERE id=?2 AND generation=?3 AND state='resolved'",
                        params![target_generation,seat.as_str(),generation],
                    ).map_err(store_error)?;
                    update_structural_proof(tx, seat, proof.as_ref().expect("validated structural proof"))?;
                    schema::bump_lifecycle_revision(tx, instance)?;
                    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
                    ReconciliationOutcome::Applied
                }
                ReconciliationAction::Move { target, terminal } => {
                    let changed = tx.execute(
                        "UPDATE seats SET target_id=?1,target_generation=?2 WHERE id=?3 AND generation=?4 AND state='resolved'",
                        params![target.as_str(),target_generation,seat.as_str(),checked_host_number(transition.expected_binding_generation)?],
                    ).map_err(store_error)?;
                    if changed != 1 {
                        return Ok(ReconciliationOutcome::Stale);
                    }
                    tx.execute(
                        "UPDATE occupant_bindings SET target_id=?1,terminal_id=?2,target_generation=?3 WHERE seat_id=?4 AND ended_at IS NULL",
                        params![target.as_str(),terminal.as_str(),target_generation,seat.as_str()],
                    ).map_err(store_error)?;
                    update_structural_proof(tx, seat, proof.as_ref().expect("validated structural proof"))?;
                    schema::bump_lifecycle_revision(tx, instance)?;
                    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
                    ReconciliationOutcome::Applied
                }
                ReconciliationAction::BeginRetirement { absent_target } => {
                    let job = control::begin_retirement_fence(
                        tx,
                        at.utc,
                        seat.clone(),
                        instance,
                        transition.publication.boot.as_str(),
                        checked_host_number(transition.publication.epoch)?,
                        absent_target.as_str(),
                        target_generation,
                    )?;
                    lift_baseline_hold_if_clear(tx, instance, at.utc)?;
                    ReconciliationOutcome::RetirementStarted(job)
                }
            };
            Ok(outcome)
        },
    )
}

/// A coherent enumeration updates observation state and establishes one durable
/// baseline. Unresolved saved seats hold every unclaimed baseline target.
/// Retained solely for legacy fixture assertions; production uses hidden stages.
#[cfg(test)]
pub fn record_snapshot(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    snapshot: HostSnapshot,
) -> Result<(), ApiError> {
    if i64::try_from(snapshot.epoch).is_err()
        || i64::try_from(snapshot.observation_sequence).is_err()
        || snapshot.targets.iter().any(|target| {
            i64::try_from(target.epoch).is_err()
                || i64::try_from(target.generation).is_err()
                || i64::try_from(target.observation_sequence).is_err()
        })
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "host evidence exceeds SQLite integer range",
        ));
    }
    if instance.is_empty()
        || !snapshot.authorizes_absence_closure()
        || snapshot.targets.iter().any(|target| {
            target.host_boot != snapshot.boot
                || target.epoch != snapshot.epoch
                || target.provenance == ObservationProvenance::UncharacterizedCache
        })
    {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "snapshot is not coherent and complete",
        ));
    }
    let mut names = std::collections::HashSet::new();
    if snapshot
        .targets
        .iter()
        .any(|target| !names.insert(target.target.as_str()))
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "duplicate target in snapshot",
        ));
    }
    context.execute_decision(conn,
        |tx| {
            type OldColumns = Option<(Option<String>, i64, i64, Option<String>, Option<i64>)>;
            let old: OldColumns = tx.query_row(
                "SELECT host_boot,host_epoch,observation_sequence,recovery_boot,recovery_epoch FROM host_instances WHERE id=?1", [instance],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            ).optional().map_err(store_error)?;
            if let Some((Some(boot), epoch, sequence, _, _)) = &old
                && *boot == snapshot.boot.as_str() {
                    if *epoch > snapshot.epoch as i64 {
                        return Err(api_error(ErrorCode::StaleHostObservation, "older host epoch"));
                    }
                    if *epoch == snapshot.epoch as i64 && *sequence >= snapshot.observation_sequence as i64 {
                        return Err(api_error(ErrorCode::StaleHostObservation, "snapshot observation sequence is not newer"));
                    }
                }
            Ok(old)
        },
        |tx, decision, old| {
            let new_baseline = old.as_ref().is_none_or(|(_, _, _, boot, _)| boot.as_deref() != Some(snapshot.boot.as_str()));
            let mut eligibility_changed = old.as_ref().is_some_and(|(boot,epoch,_,_,_)|
                boot.as_deref() != Some(snapshot.boot.as_str()) || *epoch != snapshot.epoch as i64);
            tx.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,observation_sequence,recovery_boot,recovery_epoch) VALUES (?1,?2,?3,?4,?5,?3,?4) \
                ON CONFLICT(id) DO UPDATE SET host_boot=excluded.host_boot,host_epoch=excluded.host_epoch,observation_sequence=excluded.observation_sequence,recovery_boot=excluded.recovery_boot,recovery_epoch=CASE WHEN ?6 THEN excluded.recovery_epoch ELSE host_instances.recovery_epoch END",
                params![instance, decision.utc.0, snapshot.boot.as_str(), snapshot.epoch as i64, snapshot.observation_sequence as i64, new_baseline]).map_err(store_error)?;
            let unresolved: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM seats WHERE instance_id=?1 AND state='unresolved')", [instance], |r| r.get(0)).map_err(store_error)?;
            for target in &snapshot.targets {
                let prior:Option<(String,i64,i64,i64,i64,String)>=tx.query_row(
                    "SELECT host_boot,epoch,generation,observation_sequence,observed_at,provenance FROM observed_targets WHERE instance_id=?1 AND target_id=?2",
                    params![instance,target.target.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
                    .optional().map_err(store_error)?;
                if let Some((boot,epoch,generation,sequence,observed_at,provenance))=&prior
                    && boot==snapshot.boot.as_str() && *epoch==snapshot.epoch as i64 {
                        if *sequence>target.observation_sequence as i64 {continue;}
                        if *sequence==target.observation_sequence as i64 {
                            if *generation!=target.generation as i64 || *observed_at!=target.observed_at_utc.0
                                || provenance!=provenance_name(target.provenance) {
                                return Err(api_error(ErrorCode::StaleHostObservation,"conflicting target observation sequence"));
                            }
                            continue;
                        }
                    }
                let prior_generation=prior.as_ref().map(|(_,_,generation,_,_,_)|*generation);
                if prior.as_ref().is_none_or(|(boot,epoch,generation,_,_,provenance)|
                    boot!=snapshot.boot.as_str() || *epoch!=snapshot.epoch as i64
                    || *generation!=target.generation as i64 || provenance!=provenance_name(target.provenance)) {
                    eligibility_changed = true;
                }
                tx.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
                    ON CONFLICT(instance_id,target_id) DO UPDATE SET host_boot=excluded.host_boot,epoch=excluded.epoch,generation=excluded.generation,observation_sequence=excluded.observation_sequence,observed_at=excluded.observed_at,provenance=excluded.provenance",
                    params![instance,target.target.as_str(),snapshot.boot.as_str(),snapshot.epoch as i64,target.generation as i64,
                        target.observation_sequence as i64,target.observed_at_utc.0,provenance_name(target.provenance)]).map_err(store_error)?;
                if prior_generation.is_some_and(|prior|prior!=target.generation as i64) {
                    tx.execute("UPDATE seats SET target_generation=?1 WHERE instance_id=?2 AND target_id=?3 AND state='resolved'",
                        params![target.generation as i64,instance,target.target.as_str()]).map_err(store_error)?;
                }
                let host_changed=old.as_ref().is_some_and(|(boot,epoch,_,_,_)|
                    boot.as_deref()!=Some(snapshot.boot.as_str()) || *epoch!=snapshot.epoch as i64);
                if host_changed || prior_generation.is_some_and(|prior|prior!=target.generation as i64) {
                    let current_seat:Option<String>=tx.query_row("SELECT id FROM seats WHERE instance_id=?1 AND target_id=?2 AND state='resolved'",
                        params![instance,target.target.as_str()],|r|r.get(0)).optional().map_err(store_error)?;
                    if let Some(current_seat)=current_seat {
                        schema::ensure_unavailability_episode(tx,&SeatId::new(current_seat))?;
                    }
                }
                let owned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM seats WHERE instance_id=?1 AND target_id=?2 AND state='resolved')",
                    params![instance,target.target.as_str()], |r| r.get(0)).map_err(store_error)?;
                let prior: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM recovery_baseline_targets WHERE instance_id=?1 AND target_id=?2 AND baseline_boot=?3)",
                    params![instance,target.target.as_str(),snapshot.boot.as_str()], |r| r.get(0)).map_err(store_error)?;
                if !new_baseline && prior { continue; }
                let disposition = if owned { "already_owned" } else if new_baseline && unresolved { "held_for_repair" }
                    else if new_baseline { "unambiguous_unclaimed" } else { "created_after_baseline" };
                tx.execute("INSERT INTO recovery_baseline_targets(instance_id,target_id,baseline_boot,baseline_epoch,captured_at,disposition) VALUES (?1,?2,?3,?4,?5,?6) \
                    ON CONFLICT(instance_id,target_id) DO UPDATE SET baseline_boot=excluded.baseline_boot,baseline_epoch=excluded.baseline_epoch,captured_at=excluded.captured_at,disposition=excluded.disposition",
                    params![instance,target.target.as_str(),snapshot.boot.as_str(),snapshot.epoch as i64,decision.utc.0,disposition]).map_err(store_error)?;
                if disposition == "held_for_repair" {
                    tx.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES (?1,?2,?3,?4,'unresolved saved seat continuity') \
                        ON CONFLICT(instance_id,target_id) DO UPDATE SET baseline_boot=excluded.baseline_boot,baseline_epoch=excluded.baseline_epoch,reason=excluded.reason,released_at=NULL",
                        params![instance,target.target.as_str(),snapshot.boot.as_str(),snapshot.epoch as i64]).map_err(store_error)?;
                }
            }
            schema::apply_eligibility_transition(tx,instance,|_|Ok(eligibility_changed))?;
            Ok(())
        })
}

fn target_free(tx: &Transaction<'_>, instance: &str, target: &str) -> Result<bool, ApiError> {
    tx.query_row("SELECT NOT EXISTS(SELECT 1 FROM seats WHERE instance_id=?1 AND target_id=?2 AND state='resolved')",
        params![instance,target], |r| r.get(0)).map_err(store_error)
}

fn observed_matches(
    tx: &Transaction<'_>,
    instance: &str,
    target: &str,
    boot: &HostBootId,
    epoch: u64,
    generation: u64,
) -> Result<bool, ApiError> {
    let epoch = checked_host_number(epoch)?;
    let generation = checked_host_number(generation)?;
    Ok(
        effective::effective_observation(tx, instance, target)?.is_some_and(|observation| {
            observation.source == EffectiveObservationSource::NewerCurrentTarget
                && observation.host_boot == boot.as_str()
                && observation.epoch == epoch
                && observation.structural_generation == generation
        }),
    )
}

fn structural_proof_matches_current(
    tx: &Transaction<'_>,
    instance: &str,
    proof: &DurableStructuralProof,
) -> Result<bool, ApiError> {
    let epoch = checked_host_number(proof.host_epoch())?;
    let sequence = checked_host_number(proof.observation_sequence())?;
    let generation = checked_host_number(proof.target_generation())?;
    let observation = effective::effective_observation(tx, instance, proof.target().as_str())?;
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
        "SELECT EXISTS(SELECT 1 FROM host_instances h JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.status='published' WHERE h.id=?1 AND h.host_boot=?2 AND h.host_epoch=?3 AND g.incarnation=?4)",
        params![instance, proof.host_boot().as_str(), epoch, proof.incarnation()],
        |r| r.get(0),
    ).map_err(store_error)?;
    Ok(incarnation_matches)
}

fn insert_ordinary_allocation(
    tx: &Transaction<'_>,
    at: DecisionInstant,
    instance: &str,
    proof: &DurableStructuralProof,
) -> Result<CommandResult, ApiError> {
    let seat = SeatId::new(crate::store::public_ids::fresh(
        tx,
        prefix::SEAT,
        &[("seats", "id", prefix::SEAT)],
    )?);
    tx.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence,created_at,unavailability_episode) VALUES (?1,?2,'resolved','native',?3,1,?4,?5,?6,?7,?8,?9,?10,?11,?12,1)",
                params![seat.as_str(),instance,proof.target().as_str(),checked_host_number(proof.target_generation())?,
                    proof.terminal().as_str(),proof.incarnation(),proof.source_spelling(),proof.host_boot().as_str(),
                    checked_host_number(proof.host_epoch())?,checked_host_number(proof.connection_epoch())?,
                    checked_host_number(proof.observation_sequence())?,at.utc.0]).map_err(store_error)?;
    tx.execute("INSERT INTO allocation_decisions(instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation) VALUES (?1,?2,?3,'ordinary',?4,?5,?6,?7)",
                params![instance,proof.target().as_str(),seat.as_str(),at.utc.0,proof.host_boot().as_str(),checked_host_number(proof.host_epoch())?,checked_host_number(proof.target_generation())?]).map_err(store_error)?;
    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
    schema::bump_lifecycle_revision(tx, instance)?;
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
    let admitted: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM host_instances h JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.instance_id=h.id AND g.status='published' AND g.staged_targets=g.expected_targets WHERE h.id=?1 AND h.host_boot=?2 AND h.host_epoch=?3 AND h.observation_admission_sequence=?4 AND h.observation_decided_sequence=?4 AND h.invalidation_revision=?5 AND h.active_snapshot_id=?6 AND g.incarnation=?7 AND g.published_invalidation_revision=h.invalidation_revision)",
        params![instance,proof.host_boot().as_str(),epoch,sequence,checked_host_number(admission.invalidation_revision)?,admission.expected_active.as_ref().map(SnapshotGenerationId::as_str),proof.incarnation()],
        |r| r.get(0),
    ).map_err(store_error)?;
    let effective = effective::effective_observation(tx, instance, proof.target().as_str())?;
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

fn current_resolution_owner(
    tx: &Transaction<'_>,
    instance: &str,
    guard: &OrdinaryResolutionGuard,
) -> Result<Option<SeatId>, ApiError> {
    validate_resolution_evidence(tx, instance, guard)?;
    let proof = guard.structural_proof();
    // Ownership is only a locator; independently check active explicit holds.
    let held: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE instance_id=?1 AND target_id=?2 AND released_at IS NULL)",
        params![instance,proof.target().as_str()], |r| r.get(0),
    ).map_err(store_error)?;
    if held {
        return Err(api_error(
            ErrorCode::TargetUnresolved,
            "resolution target is held for repair",
        ));
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
        "SELECT id,target_generation,structural_terminal_id,structural_incarnation,structural_host_boot,structural_host_epoch,retired_at,retired_seq FROM seats WHERE instance_id=?1 AND target_id=?2 AND state='resolved' LIMIT 1",
        params![instance,proof.target().as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)),
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

fn validate_resolution_allocation(
    tx: &Transaction<'_>,
    instance: &str,
    proof: &DurableStructuralProof,
) -> Result<(), ApiError> {
    let recovery: Option<(Option<String>, Option<i64>)> = tx
        .query_row(
            "SELECT recovery_boot,recovery_epoch FROM host_instances WHERE id=?1",
            [instance],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let clear = matches!(
        effective::effective_recovery_disposition(tx, instance, proof.target().as_str())?,
        EffectiveRecoveryDisposition::UnambiguousUnclaimed
            | EffectiveRecoveryDisposition::CreatedAfterBaseline
    );
    if !clear
        || !recovery.is_some_and(|(boot, epoch)| {
            boot.as_deref() == Some(proof.host_boot().as_str())
                && epoch.is_some_and(|epoch| epoch >= 0 && epoch <= proof.host_epoch() as i64)
        })
    {
        return Err(api_error(
            ErrorCode::TargetUnresolved,
            "target lacks a clear durable recovery baseline",
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
                "SELECT lifecycle_revision FROM host_instances WHERE id=?1",
                [instance],
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
        validate_resolution_allocation(tx, instance, guard.structural_proof())?;
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
            if current_resolution_owner(tx, instance, &guard)?.is_none() {
                // A previously unowned target can allocate only before a known
                // lifecycle change after this explicit read. A concurrent new
                // allocation is handled by the valid existing-owner branch.
                let lifecycle: i64 = tx
                    .query_row(
                        "SELECT lifecycle_revision FROM host_instances WHERE id=?1",
                        [instance],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                let published_lifecycle =
                    checked_host_number(guard.admission().lifecycle_revision)?
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
                validate_resolution_allocation(tx, instance, guard.structural_proof())?;
            }
            Ok(())
        },
        |tx, at| {
            snapshot_budget(context, budget)?;
            let owner: Option<String> = tx.query_row(
                "SELECT id FROM seats WHERE instance_id=?1 AND target_id=?2 AND state='resolved' LIMIT 1",
                params![instance,request.target.as_str()], |r| r.get(0),
            ).optional().map_err(store_error)?;
            match owner {
                Some(seat) => Ok(CommandResult::SeatResolved(SeatId::new(seat))),
                None => insert_ordinary_allocation(tx, at, instance, guard.structural_proof()),
            }
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
                    "SELECT lifecycle_revision FROM host_instances WHERE id=?1",
                    [instance],
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

/// Explicit operator repair still requires a per-request target observation.
pub fn mutate_operator(
    context: &StoreContext,
    conn: &mut Connection,
    elected_instance: &str,
    mut request: OperatorRequest,
    actor: OperatorActor,
    installation_default_ms: Option<u64>,
) -> Result<CommandResult, ApiError> {
    let (
        kind,
        target,
        operation,
        instance,
        expected_boot,
        expected_epoch,
        expected_generation,
        seat,
        structural_proof,
    ) = match &request {
        OperatorRequest::FreshSeat(command, guard) => (
            "operator_fresh",
            command.target.clone(),
            command.operation.clone(),
            guard.instance().to_owned(),
            guard.host_boot().clone(),
            guard.epoch(),
            guard.generation(),
            None,
            guard.structural_proof().cloned(),
        ),
        OperatorRequest::Rebind(command, guard) => (
            "operator_rebind",
            command.target.clone(),
            command.operation.clone(),
            guard.instance().to_owned(),
            guard.host_boot().clone(),
            guard.epoch(),
            guard.generation(),
            Some(command.seat.clone()),
            guard.structural_proof().cloned(),
        ),
        OperatorRequest::Replace(command, guard) => (
            "operator_rebind",
            command.target.clone(),
            command.operation.clone(),
            guard.instance().to_owned(),
            guard.host_boot().clone(),
            guard.epoch(),
            guard.generation(),
            Some(command.seat.clone()),
            guard.structural_proof().cloned(),
        ),
        OperatorRequest::OrphanInvite(command) => {
            return crate::store::control::operator_orphan_invite(
                context,
                conn,
                elected_instance,
                command,
                actor,
                installation_default_ms,
            );
        }
        OperatorRequest::Retire(command) => {
            return operator_retire(context, conn, elected_instance, command, actor);
        }
    };
    let replace: Option<SeatId> = match &request {
        OperatorRequest::Replace(command, _) => Some(command.replace.clone()),
        _ => None,
    };
    if instance != elected_instance {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "operator guard belongs to another instance",
        ));
    }
    let typed_command = match &request {
        OperatorRequest::FreshSeat(c, _) => {
            crate::protocol::commands::OperatorCommand::FreshSeat(c.clone())
        }
        OperatorRequest::Rebind(c, _) => {
            crate::protocol::commands::OperatorCommand::Rebind(c.clone())
        }
        OperatorRequest::Replace(c, _) => {
            crate::protocol::commands::OperatorCommand::Replace(c.clone())
        }
        OperatorRequest::OrphanInvite(_) | OperatorRequest::Retire(_) => unreachable!(),
    };
    let expected_epoch_sql = checked_host_number(expected_epoch)?;
    let expected_generation_sql = checked_host_number(expected_generation)?;
    let digest = crate::store::operator::digest(elected_instance, &typed_command)?;
    let result = schema::execute_idempotent_transaction(
        context,
        conn,
        &actor.operation_scope(&instance),
        operation.as_str(),
        digest,
        |tx| {
            if !observed_matches(
                tx,
                &instance,
                target.as_str(),
                &expected_boot,
                expected_epoch,
                expected_generation,
            )? {
                return Err(api_error(
                    ErrorCode::TargetAlreadyOwned,
                    "repair target changed or owned",
                ));
            }
            if let Some(replace) = &replace {
                // The target is owned by NEW by design; it must be NEW and
                // nobody else, and NEW must not be the seat being rebound.
                let owned: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2 AND state='resolved' AND target_id=?3)",
                        params![replace.as_str(), instance, target.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                if !owned || seat.as_ref() == Some(replace) {
                    return Err(api_error(
                        ErrorCode::TargetAlreadyOwned,
                        "replace target is not owned by NEW",
                    ));
                }
            } else if !target_free(tx, &instance, target.as_str())? {
                return Err(owned_target_refusal(
                    tx,
                    &instance,
                    target.as_str(),
                    seat.as_ref(),
                )?);
            }
            if let Some(proof) = &structural_proof
                && !structural_proof_matches_current(tx, &instance, proof)?
            {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "repair structural proof differs from current evidence",
                ));
            }
            if let Some(seat) = &seat {
                let state: Option<(String, i64)> = tx
                    .query_row(
                        "SELECT state,generation FROM seats WHERE id=?1 AND instance_id=?2",
                        params![seat.as_str(), instance],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .map_err(store_error)?;
                if state.as_ref().map(|(state, _)| state.as_str()) != Some("unresolved") {
                    return Err(api_error(
                        ErrorCode::TargetUnresolved,
                        "rebind source is not unresolved",
                    ));
                }
                if state.is_some_and(|(_, generation)| generation == i64::MAX) {
                    return Err(api_error(
                        ErrorCode::SequenceExhausted,
                        "seat binding generation exhausted",
                    ));
                }
            }
            Ok(())
        },
        |tx, at| {
            let fresh = effective::effective_observation(tx, &instance, target.as_str())?
                .filter(|observation| {
                    observation.source == EffectiveObservationSource::NewerCurrentTarget
                })
                .ok_or_else(|| {
                    api_error(
                        ErrorCode::StaleHostObservation,
                        "fresh target observation missing",
                    )
                })?;
            request
                .consume_for_decision(
                    &instance,
                    &DecisionFence {
                        now: at.monotonic,
                        host_boot: HostBootId::new(fresh.host_boot),
                        host_epoch: u64::try_from(fresh.epoch).map_err(|_| {
                            api_error(ErrorCode::StoreCorrupt, "negative host epoch")
                        })?,
                        target_generation: u64::try_from(fresh.structural_generation).map_err(
                            |_| api_error(ErrorCode::StoreCorrupt, "negative target generation"),
                        )?,
                        binding_generation: 0,
                        known_invalidated: !observed_matches(
                            tx,
                            &instance,
                            target.as_str(),
                            &expected_boot,
                            expected_epoch,
                            expected_generation,
                        )?,
                    },
                )
                .map_err(|reason| api_error(ErrorCode::StaleHostObservation, reason))?;
            if let Some(replace) = &replace {
                // One deciding transaction: NEW retires and OLD claims the
                // target with no unowned window; a concurrent resolve
                // serializes behind this writer and sees OLD.
                let (boot, epoch, new_generation): (Option<String>, i64, i64) = tx
                    .query_row(
                        "SELECT h.host_boot,h.host_epoch,s.generation FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1 AND s.instance_id=?2",
                        params![replace.as_str(), instance],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .map_err(store_error)?;
                if new_generation == i64::MAX {
                    return Err(api_error(
                        ErrorCode::SequenceExhausted,
                        "seat binding generation exhausted",
                    ));
                }
                let boot = boot.unwrap_or_default();
                control::begin_retirement_fence(
                    tx,
                    at.utc,
                    replace.clone(),
                    &instance,
                    &boot,
                    epoch,
                    target.as_str(),
                    expected_generation_sql,
                )?;
                tx.execute("INSERT INTO allocation_decisions(instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES (?1,?2,?3,'operator_retire',?4,?5,?6,?7,?8)",
                    params![instance,target.as_str(),replace.as_str(),at.utc.0,boot,epoch,expected_generation_sql,actor.audit_label()]).map_err(store_error)?;
            }
            if !target_free(tx, &instance, target.as_str())? {
                return Err(api_error(
                    ErrorCode::TargetAlreadyOwned,
                    "target became owned",
                ));
            }
            let result = if let Some(seat) = &seat {
                rebind_unresolved_seat(
                    tx,
                    &instance,
                    seat,
                    target.as_str(),
                    expected_generation_sql,
                    at.utc,
                )?;
                CommandResult::OperatorRebound(seat.clone())
            } else {
                let seat = SeatId::new(crate::store::public_ids::fresh(
                    tx,
                    prefix::SEAT,
                    &[("seats", "id", prefix::SEAT)],
                )?);
                tx.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES (?1,?2,'resolved','operator_fresh',?3,1,?4,?5,1)",
                    params![seat.as_str(),instance,target.as_str(),expected_generation_sql,at.utc.0]).map_err(store_error)?;
                CommandResult::OperatorFreshSeat(seat)
            };
            let result_seat = match &result {
                CommandResult::OperatorRebound(id) | CommandResult::OperatorFreshSeat(id) => id,
                _ => unreachable!(),
            };
            if let Some(proof) = &structural_proof {
                update_structural_proof(tx, result_seat, proof)?;
            } else {
                // Operator mapping without structural evidence cannot carry a prior proof.
                tx.execute("UPDATE seats SET structural_terminal_id=NULL,structural_incarnation=NULL,structural_incarnation_kind=NULL,structural_host_boot=NULL,structural_host_epoch=NULL,structural_connection_epoch=NULL,structural_observation_sequence=NULL WHERE id=?1", [result_seat.as_str()]).map_err(store_error)?;
            }
            tx.execute("INSERT INTO allocation_decisions(instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![instance,target.as_str(),result_seat.as_str(),kind,at.utc.0,expected_boot.as_str(),expected_epoch_sql,expected_generation_sql,actor.audit_label()]).map_err(store_error)?;
            release_claimed_target(tx, &instance, target.as_str(), at.utc)?;
            lift_baseline_hold_if_clear(tx, &instance, at.utc)?;
            Ok(result)
        },
    )?;
    crate::store::operator::validate_result(&typed_command, &result)?;
    Ok(result)
}

/// The seat-side half of a rebind of an unresolved seat onto `target`: one
/// generation step, the open binding ended, its warning offer dropped and an
/// unavailability episode open. Operator repair leaves the successor binding to
/// the agent's next check-in; TRUST-POLICY C1 continuity opens it in the same
/// transaction (`decide_continuity`).
fn rebind_unresolved_seat(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    target: &str,
    target_generation: i64,
    at: UtcMillis,
) -> Result<(), ApiError> {
    let changed = tx.execute("UPDATE seats SET state='resolved',unresolved_reason=NULL,unresolved_from_generation_id=NULL,unresolved_prior_binding_generation=NULL,target_id=?1,generation=generation+1,target_generation=?2 WHERE id=?3 AND instance_id=?4 AND state='unresolved'",
        params![target,target_generation,seat.as_str(),instance]).map_err(store_error)?;
    if changed != 1 {
        return Err(api_error(ErrorCode::TargetUnresolved, "source changed"));
    }
    tx.execute(
        "UPDATE occupant_bindings SET ended_at=?1 WHERE seat_id=?2 AND ended_at IS NULL",
        params![at.0, seat.as_str()],
    )
    .map_err(store_error)?;
    catch_up::supersede_stale(tx, seat, at)?;
    tx.execute(
        "DELETE FROM warning_offer WHERE seat_id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    schema::ensure_unavailability_episode(tx, seat)?;
    Ok(())
}

/// The target-side half of claiming `target` for a seat: its recovery holds
/// are released, its baseline membership recorded as released and owned, and
/// eligibility and revisions advance. Shared by operator repair and C1.
fn release_claimed_target(
    tx: &Transaction<'_>,
    instance: &str,
    target: &str,
    at: UtcMillis,
) -> Result<(), ApiError> {
    tx.execute("UPDATE recovery_holds SET released_at=?1 WHERE instance_id=?2 AND target_id=?3 AND released_at IS NULL",
        params![at.0,instance,target]).map_err(store_error)?;
    let baseline: Option<String> = tx
        .query_row(
            "SELECT recovery_baseline_generation_id FROM host_instances WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if let Some(baseline) = baseline {
        let member: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM snapshot_targets WHERE generation_id=?1 AND target_id=?2)",
                params![baseline,target],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if member {
            let sequence = schema::next_decision_seq(tx, instance)?;
            tx.execute(
                "INSERT INTO recovery_baseline_releases(instance_id,baseline_generation_id,target_id,decision_seq) VALUES (?1,?2,?3,?4) ON CONFLICT(instance_id,baseline_generation_id,target_id) DO NOTHING",
                params![instance,baseline,target,checked_host_number(sequence)?],
            )
            .map_err(store_error)?;
        }
    }
    tx.execute("UPDATE recovery_baseline_targets SET disposition='already_owned' WHERE instance_id=?1 AND target_id=?2",
        params![instance,target]).map_err(store_error)?;
    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
    schema::bump_lifecycle_revision(tx, instance)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    Ok(())
}

/// Whether the first reconciliation pass of the current recovery epoch has
/// not run yet: seats a Herdr restart will unresolve may still look resolved,
/// and unresolved seats it will rebuild may not exist yet.
fn reconciliation_lags(tx: &Transaction<'_>, instance: &str) -> Result<bool, ApiError> {
    Ok(tx
        .query_row(
            "SELECT recovery_boot IS NOT reconciled_boot OR recovery_epoch IS NOT reconciled_epoch FROM host_instances WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .unwrap_or(false))
}

/// TRUST-POLICY C1 candidates: nonretired unresolved seats of the instance
/// whose *latest* binding is this harness's session `native_session`.
/// Sentinel (`plugin_context:`), empty and human values never match, and a
/// latest `managed_launch` binding (no session: the agent never checked in)
/// makes the seat no candidate (ht-5n6); NULL
/// never equals anything. Returns at most two (the caller only needs to tell
/// zero, one and several apart).
pub(crate) fn continuity_candidates(
    tx: &Transaction<'_>,
    instance: &str,
    harness: &str,
    native_session: &str,
) -> Result<Vec<SeatId>, ApiError> {
    let mut statement = tx
        .prepare(
            "SELECT s.id FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.ordinal=(SELECT MAX(l.ordinal) FROM occupant_bindings l WHERE l.seat_id=s.id) WHERE s.instance_id=?1 AND s.state='unresolved' AND b.harness=?2 AND b.harness<>'human' AND b.observation_provenance<>'managed_launch' AND b.native_session=?3 AND b.native_session<>'' AND substr(b.native_session,1,15)<>'plugin_context:' ORDER BY s.id LIMIT 2",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map(params![instance, harness, native_session], |r| {
            r.get::<_, String>(0)
        })
        .map_err(store_error)?;
    rows.map(|row| row.map(SeatId::new).map_err(store_error))
        .collect()
}

pub fn continuity_digest(
    instance: &str,
    command: &crate::protocol::commands::ContinuityCheckIn,
) -> Result<[u8; 32], ApiError> {
    schema::canonical_digest(&(
        "continuity_check_in",
        instance,
        &command.target,
        &command.harness,
        &command.native_session,
        &command.execution,
    ))
}

fn continuity_scope(instance: &str) -> String {
    format!("continuity:{instance}")
}

fn validate_continuity_result(result: &CommandResult) -> Result<(), ApiError> {
    if matches!(result, CommandResult::ContinuityReattached(_)) {
        Ok(())
    } else {
        Err(api_error(
            ErrorCode::StoreCorrupt,
            "continuity replay result has wrong type",
        ))
    }
}

/// Historical continuity result for the same operation key and payload. Grants
/// no current authority and touches no durable state.
pub fn replay_continuity(
    context: &StoreContext,
    instance: &str,
    command: &crate::protocol::commands::ContinuityCheckIn,
    budget: &CallBudget,
) -> Result<Option<CommandResult>, ApiError> {
    let expected = continuity_digest(instance, command)?;
    let db = context.open_query(budget.clone())?;
    let stored: Option<(Vec<u8>, String)> = db
        .query_row(
            "SELECT digest, result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![continuity_scope(instance), command.operation.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| db.map_error(e))?;
    let Some((digest, json)) = stored else {
        return Ok(None);
    };
    if digest != expected {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "operation key reused with different payload",
        ));
    }
    let result: CommandResult = serde_json::from_str(&json).map_err(|e| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("invalid stored result: {e}"),
        )
    })?;
    validate_continuity_result(&result)?;
    Ok(Some(result))
}

/// TRUST-POLICY C1: reattach the one unresolved seat whose last binding holds
/// the resumed session id onto the (held or unowned) target, in one deciding
/// transaction that is a complete lifecycle check-in. The match is cooperative
/// and structural only: Herdr's `agent_session` (`diagnostic`) is recorded,
/// never consulted. The seat's old binding is ended and the successor
/// `cooperative_top_level` binding (the command's harness, session and
/// execution, a new generation) is opened with its availability anchor in the
/// same transaction; the reply is idempotent under the operation key and a lost
/// reply is recovered by the committed binding. The `cooperative_continuity`
/// value is written to seat history (`allocation_decisions`) only, never to a
/// receipt or the binding. With no matching unresolved seat, or a target that
/// still has a resolved owner, while the host epoch's reconciliation lags its
/// recovery marker, the refusal is the retryable `ServiceBusy`, not `NotFound`
/// or `TargetAlreadyOwned`.
pub fn decide_continuity(
    context: &StoreContext,
    conn: &mut Connection,
    elected_instance: &str,
    mut request: crate::ports::ContinuityRequest,
) -> Result<CommandResult, ApiError> {
    let command = request.command.clone();
    let diagnostic = request.diagnostic;
    let instance = request.guard.instance().to_owned();
    if instance != elected_instance {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "continuity guard belongs to another instance",
        ));
    }
    command
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    let target = command.target.clone();
    let expected_boot = request.guard.host_boot().clone();
    let expected_epoch = request.guard.epoch();
    let expected_generation = request.guard.generation();
    let structural_proof = request.guard.structural_proof().cloned();
    let expected_epoch_sql = checked_host_number(expected_epoch)?;
    let expected_generation_sql = checked_host_number(expected_generation)?;
    let digest = continuity_digest(elected_instance, &command)?;
    let harness = command.harness.as_str();
    let session = command.native_session.as_str();
    let result = schema::execute_idempotent_transaction(
        context,
        conn,
        &continuity_scope(&instance),
        command.operation.as_str(),
        digest,
        |tx| {
            if !observed_matches(
                tx,
                &instance,
                target.as_str(),
                &expected_boot,
                expected_epoch,
                expected_generation,
            )? {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "continuity target observation changed",
                ));
            }
            // Held or unowned only: a target with a resolved owner is never
            // taken (seats are never merged, nothing moves on a heuristic).
            // Until the first reconciliation pass of the current recovery
            // epoch has run, the owner may be a mapping from the previous
            // Herdr incarnation that the pass will unresolve (a restored
            // pane that kept its id, ht-p63): pending, not final.
            if !target_free(tx, &instance, target.as_str())? {
                return Err(if reconciliation_lags(tx, &instance)? {
                    api_error(
                        ErrorCode::ServiceBusy,
                        "recovery reconciliation of the current host epoch has not finished; retry",
                    )
                } else {
                    api_error(
                        ErrorCode::TargetAlreadyOwned,
                        "resumed session's pane already has a resolved seat",
                    )
                });
            }
            if let Some(proof) = &structural_proof
                && !structural_proof_matches_current(tx, &instance, proof)?
            {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "continuity structural proof differs from current evidence",
                ));
            }
            let candidates = continuity_candidates(tx, &instance, harness, session)?;
            let [seat] = candidates.as_slice() else {
                return Err(if candidates.is_empty() {
                    // Until the first reconciliation pass of the current
                    // recovery epoch has run, "no unresolved seat" only says the
                    // daemon has not yet rebuilt the seats a Herdr restart
                    // unresolved: pending, not final.
                    if reconciliation_lags(tx, &instance)? {
                        api_error(
                            ErrorCode::ServiceBusy,
                            "recovery reconciliation of the current host epoch has not finished; retry",
                        )
                    } else {
                        api_error(
                            ErrorCode::NotFound,
                            "no unresolved seat matches this resumed session",
                        )
                    }
                } else {
                    api_error(
                        ErrorCode::Conflict,
                        "resumed session matches several unresolved seats; repair with seat rebind --operator",
                    )
                });
            };
            let generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1 AND instance_id=?2",
                    params![seat.as_str(), instance],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if generation == i64::MAX {
                return Err(api_error(
                    ErrorCode::SequenceExhausted,
                    "seat binding generation exhausted",
                ));
            }
            Ok(())
        },
        |tx, at| {
            let fresh = effective::effective_observation(tx, &instance, target.as_str())?
                .filter(|observation| {
                    observation.source == EffectiveObservationSource::NewerCurrentTarget
                })
                .ok_or_else(|| {
                    api_error(
                        ErrorCode::StaleHostObservation,
                        "fresh target observation missing",
                    )
                })?;
            request
                .guard
                .consume(
                    &instance,
                    &command,
                    &DecisionFence {
                        now: at.monotonic,
                        host_boot: HostBootId::new(fresh.host_boot),
                        host_epoch: u64::try_from(fresh.epoch).map_err(|_| {
                            api_error(ErrorCode::StoreCorrupt, "negative host epoch")
                        })?,
                        target_generation: u64::try_from(fresh.structural_generation).map_err(
                            |_| api_error(ErrorCode::StoreCorrupt, "negative target generation"),
                        )?,
                        binding_generation: 0,
                        known_invalidated: !observed_matches(
                            tx,
                            &instance,
                            target.as_str(),
                            &expected_boot,
                            expected_epoch,
                            expected_generation,
                        )?,
                    },
                )
                .map_err(|reason| api_error(ErrorCode::StaleHostObservation, reason))?;
            let candidates = continuity_candidates(tx, &instance, harness, session)?;
            let [seat] = candidates.as_slice() else {
                return Err(api_error(ErrorCode::Conflict, "continuity match changed"));
            };
            if !target_free(tx, &instance, target.as_str())? {
                return Err(api_error(
                    ErrorCode::TargetAlreadyOwned,
                    "target became owned",
                ));
            }
            rebind_unresolved_seat(
                tx,
                &instance,
                seat,
                target.as_str(),
                expected_generation_sql,
                at.utc,
            )?;
            if let Some(proof) = &structural_proof {
                update_structural_proof(tx, seat, proof)?;
            } else {
                tx.execute("UPDATE seats SET structural_terminal_id=NULL,structural_incarnation=NULL,structural_incarnation_kind=NULL,structural_host_boot=NULL,structural_host_epoch=NULL,structural_connection_epoch=NULL,structural_observation_sequence=NULL WHERE id=?1", [seat.as_str()]).map_err(store_error)?;
            }
            let new_generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1",
                    [seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            // The successor binding opens here, so the reply is a complete
            // check-in: the seat is resolved, bound and available when this
            // transaction commits. Evidence is the structural proof when the
            // guard carries one, else the fresh observation of the same target.
            let (terminal, incarnation) = match &structural_proof {
                Some(proof) => (
                    proof.terminal().as_str().to_owned(),
                    proof.incarnation().to_owned(),
                ),
                None => {
                    let (terminal, incarnation) = binding_evidence(
                        fresh.terminal_id.as_deref(),
                        fresh.incarnation.as_deref(),
                    )?;
                    (terminal.to_owned(), incarnation.to_owned())
                }
            };
            tx.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11,?12,?13)",
                params![seat.as_str(),new_generation,target.as_str(),expected_boot.as_str(),expected_epoch_sql,expected_generation_sql,
                    command.harness.as_str(),command.native_session.as_str(),command.execution.as_str(),
                    crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE,at.utc.0,terminal,incarnation]).map_err(store_error)?;
            catch_up::supersede_stale(tx, seat, at.utc)?;
            tx.execute("UPDATE wake_work SET binding_generation=?1 WHERE seat_id=?2 AND reservation_id IS NULL",
                params![new_generation,seat.as_str()]).map_err(store_error)?;
            let seq = schema::next_decision_seq(tx, &instance)?;
            anchor_seat_availability(
                tx,
                seat,
                seq,
                at.utc,
                new_generation,
                crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE,
            )?;
            schema::clear_open_unavailability_for_seat(tx, seat.as_str(), at.utc)?;
            tx.execute(
                "UPDATE seats SET unavailability_open=0 WHERE id=?1",
                [seat.as_str()],
            )
            .map_err(store_error)?;
            tx.execute("INSERT INTO allocation_decisions(instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label,continuity_diagnostic) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,NULL,?9)",
                params![instance,target.as_str(),seat.as_str(),crate::protocol::authority::COOPERATIVE_CONTINUITY_PROVENANCE,at.utc.0,expected_boot.as_str(),expected_epoch_sql,expected_generation_sql,diagnostic]).map_err(store_error)?;
            release_claimed_target(tx, &instance, target.as_str(), at.utc)?;
            lift_baseline_hold_if_clear(tx, &instance, at.utc)?;
            Ok(CommandResult::ContinuityReattached(
                crate::protocol::results::ContinuityReattachment {
                    seat: seat.clone(),
                    binding_generation: u64::try_from(new_generation).map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "negative seat generation")
                    })?,
                },
            ))
        },
    )?;
    validate_continuity_result(&result)?;
    Ok(result)
}

/// TRUST-POLICY C3: the refusal for a rebind onto an owned target names both
/// operator resolutions. Falls back to the plain message when the owner is not
/// a live seat of this instance (the observation, not a seat, is then stale).
fn owned_target_refusal(
    tx: &Transaction<'_>,
    instance: &str,
    target: &str,
    old: Option<&SeatId>,
) -> Result<ApiError, ApiError> {
    let owner: Option<String> = tx
        .query_row(
            "SELECT id FROM seats WHERE instance_id=?1 AND target_id=?2 AND state='resolved' LIMIT 1",
            params![instance, target],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(match (owner, old) {
        (Some(new), Some(old)) => api_error(
            ErrorCode::TargetAlreadyOwned,
            format!(
                "target {target} is owned by live seat {new}; seats are never merged. Abandon the old seat: `herdr-threads seat retire {old} --operator`, or abandon the new role: `herdr-threads seat rebind {old} --pane {target} --replace {new} --operator`",
                old = old.as_str()
            ),
        ),
        _ => api_error(
            ErrorCode::TargetAlreadyOwned,
            "repair target changed or owned",
        ),
    })
}

/// `seat retire SEAT --operator`: the existing bounded retirement cutover,
/// started on the operator's say-so. Retirement claims no target, so there is
/// no host observation; pending obligations settle as recipient-retired when
/// the retirement worker advances the job.
fn operator_retire(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    command: &crate::protocol::commands::OperatorRetire,
    actor: OperatorActor,
) -> Result<CommandResult, ApiError> {
    let typed = crate::protocol::commands::OperatorCommand::Retire(command.clone());
    let digest = crate::store::operator::digest(instance, &typed)?;
    let seat = command.seat.clone();
    let result = schema::execute_idempotent_transaction(
        context,
        conn,
        &actor.operation_scope(instance),
        command.operation.as_str(),
        digest,
        |tx| {
            let state: Option<(String, i64)> = tx
                .query_row(
                    "SELECT state,generation FROM seats WHERE id=?1 AND instance_id=?2",
                    params![seat.as_str(), instance],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            match state {
                Some((state, generation)) if state != "retired" => {
                    if generation == i64::MAX {
                        return Err(api_error(
                            ErrorCode::SequenceExhausted,
                            "seat binding generation exhausted",
                        ));
                    }
                    Ok(())
                }
                _ => Err(api_error(
                    ErrorCode::NotFound,
                    "seat is not a live seat of this instance",
                )),
            }
        },
        |tx, at| {
            let (boot, epoch): (Option<String>, i64) = tx
                .query_row(
                    "SELECT host_boot,host_epoch FROM host_instances WHERE id=?1",
                    [instance],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(store_error)?;
            let (target, generation): (Option<String>, i64) = tx
                .query_row(
                    "SELECT target_id,target_generation FROM seats WHERE id=?1",
                    [seat.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(store_error)?;
            let target = match target {
                Some(target) => target,
                None => tx
                    .query_row(
                        "SELECT target_id FROM occupant_bindings WHERE seat_id=?1 ORDER BY ordinal DESC LIMIT 1",
                        [seat.as_str()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(store_error)?
                    .unwrap_or_default(),
            };
            let boot = boot.unwrap_or_default();
            control::begin_retirement_fence(
                tx,
                at.utc,
                seat.clone(),
                instance,
                &boot,
                epoch,
                &target,
                generation,
            )?;
            tx.execute("INSERT INTO allocation_decisions(instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES (?1,?2,?3,'operator_retire',?4,?5,?6,?7,?8)",
                params![instance,target,seat.as_str(),at.utc.0,boot,epoch,generation,actor.audit_label()]).map_err(store_error)?;
            lift_baseline_hold_if_clear(tx, instance, at.utc)?;
            Ok(CommandResult::OperatorRetired(seat.clone()))
        },
    )?;
    crate::store::operator::validate_result(&typed, &result)?;
    Ok(result)
}

/// Local mapping witness. These boot/epoch values describe durable mapping;
/// they are never native caller provenance or execution attestation.
#[derive(Debug, Clone)]
pub(crate) struct CooperativeMapping {
    pub boot: String,
    pub epoch: u64,
    pub revision: u64,
    pub invalidation_revision: u64,
    pub generation: u64,
    /// The claim's binding generation: `generation`, or one less when a
    /// lifecycle check-in replaces a `managed_launch` binding recorded after
    /// it was prepared (ht-5n6).
    pub claimed_generation: u64,
    /// Terminal and host incarnation from the same verified effective
    /// observation that proved this mapping. A binding written from this
    /// mapping stores them so snapshot reconciliation can later reconfirm it.
    pub terminal: Option<String>,
    pub incarnation: Option<String>,
}

impl CooperativeMapping {
    /// The reconfirmation evidence a new or re-registered binding must carry.
    pub(crate) fn reconfirmation_evidence(&self) -> Result<(&str, &str), ApiError> {
        binding_evidence(self.terminal.as_deref(), self.incarnation.as_deref())
    }
}

/// A binding is stored only with the terminal and host incarnation that a
/// later complete snapshot must match to reconfirm it after invalidation.
/// Without them the seat could never recover automatically, so registration
/// fails closed and actionable instead of storing an unrecoverable binding.
pub(crate) fn binding_evidence<'a>(
    terminal: Option<&'a str>,
    incarnation: Option<&'a str>,
) -> Result<(&'a str, &'a str), ApiError> {
    match (terminal, incarnation) {
        (Some(terminal), Some(incarnation)) if !terminal.is_empty() && !incarnation.is_empty() => {
            Ok((terminal, incarnation))
        }
        _ => Err(api_error(
            ErrorCode::StaleHostObservation,
            "host observation lacks terminal/incarnation reconfirmation evidence; retry after the next verified host observation",
        )),
    }
}

pub(crate) fn cooperative_instance(
    db: &Connection,
    instance: &str,
    claim: &crate::protocol::authority::CallerClaim,
) -> Result<(), ApiError> {
    if claim.instance != instance || claim.role != crate::protocol::authority::CallerRole::TopLevel
    {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative instance or declared role mismatch",
        ));
    }
    let owner: Option<String> = db
        .query_row(
            "SELECT instance_id FROM seats WHERE id=?1",
            [claim.seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if owner.as_deref() != Some(instance) {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative seat belongs to another instance",
        ));
    }
    Ok(())
}

pub(crate) fn cooperative_mapping(
    db: &Connection,
    claim: &crate::protocol::authority::CallerClaim,
    mode: Option<crate::protocol::commands::CheckInMode>,
) -> Result<CooperativeMapping, ApiError> {
    cooperative_instance(db, &claim.instance, claim)?;
    if !uuid::Uuid::parse_str(claim.execution.as_str())
        .is_ok_and(|execution| execution.hyphenated().to_string() == claim.execution.as_str())
    {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative execution must be a canonical lowercase hyphenated UUID",
        ));
    }
    type RowColumns = Option<(String, Option<String>, i64, i64, Option<String>, i64, i64)>;
    let row: RowColumns = db.query_row(
        "SELECT s.state,s.target_id,s.generation,s.target_generation,h.host_boot,h.host_epoch,h.invalidation_revision FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1",
        [claim.seat.as_str()], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(store_error)?;
    let Some((state, target, generation, revision, boot, epoch, invalidation)) = row else {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative seat missing",
        ));
    };
    if state != "resolved" || target.as_deref() != Some(claim.target.as_str()) {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative mapping or generation changed",
        ));
    }
    let lifecycle_mode = matches!(
        mode,
        Some(crate::protocol::commands::CheckInMode::Lifecycle { .. })
    );
    let claimed = checked_host_number(claim.binding_generation)?;
    // TRUST-POLICY A4 (ht-5n6): a lifecycle check-in prepared against the
    // seat generation just before `launch` recorded its `managed_launch`
    // binding (exactly one generation earlier) still replaces it: the agent
    // that launch started checked in while launch was reporting it.
    let behind_launch = lifecycle_mode
        && claimed.checked_add(1) == Some(generation)
        && db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id=?1 AND generation=?2 AND ended_at IS NULL AND observation_provenance=?3)",
                params![
                    claim.seat.as_str(),
                    generation,
                    crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE
                ],
                |r| r.get::<_, bool>(0),
            )
            .map_err(store_error)?;
    if generation != claimed && !behind_launch {
        let code = if lifecycle_mode {
            ErrorCode::Conflict
        } else {
            ErrorCode::CallerUnverified
        };
        return Err(api_error(code, "cooperative binding generation changed"));
    }
    let held: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE instance_id=?1 AND target_id=?2 AND released_at IS NULL)",
        params![claim.instance,claim.target.as_str()], |r|r.get(0)).map_err(store_error)?;
    let observation = effective::effective_observation(db, &claim.instance, claim.target.as_str())?;
    if held
        || observation.as_ref().is_none_or(|o| {
            o.structural_generation != revision
                || Some(o.host_boot.as_str()) != boot.as_deref()
                || o.epoch != epoch
        })
    {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "mapping held or known invalidated",
        ));
    }
    match mode {
        Some(crate::protocol::commands::CheckInMode::Lifecycle {
            expected_binding_generation,
        }) => {
            if expected_binding_generation != claim.binding_generation {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "lifecycle expected generation differs from context",
                ));
            }
            let used: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id=?1 AND execution_id=?2)",
                params![claim.seat.as_str(),claim.execution.as_str()], |r|r.get(0)).map_err(store_error)?;
            if used {
                return Err(api_error(
                    ErrorCode::CallerUnverified,
                    "lifecycle execution UUID already used",
                ));
            }
        }
        _ => {
            // A `managed_launch` binding is never a caller's context (A2/A3):
            // only a lifecycle check-in replaces it.
            let exact: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id=?1 AND generation=?2 AND target_id=?3 AND harness=?4 AND native_session=?5 AND execution_id=?6 AND ended_at IS NULL AND observation_provenance<>?7)",
                params![claim.seat.as_str(),generation,claim.target.as_str(),claim.harness.as_str(),claim.native_session.as_str(),claim.execution.as_str(),crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE], |r|r.get(0)).map_err(store_error)?;
            if !exact {
                return Err(api_error(
                    ErrorCode::CallerUnverified,
                    "cooperative occupant context changed",
                ));
            }
        }
    }
    Ok(CooperativeMapping {
        boot: boot.unwrap(),
        epoch: epoch as u64,
        revision: revision as u64,
        invalidation_revision: invalidation as u64,
        generation: generation as u64,
        claimed_generation: claimed as u64,
        terminal: observation.as_ref().and_then(|o| o.terminal_id.clone()),
        incarnation: observation.and_then(|o| o.incarnation),
    })
}

pub(crate) fn issue_cooperative_permit(
    context: &StoreContext,
    db: &Connection,
    instance: &str,
    mut request: crate::ports::CooperativePermitRequest,
    budget: &CallBudget,
) -> Result<MutationPermit, ApiError> {
    cooperative_instance(db, instance, &request.claim)?;
    let stored: Option<Vec<u8>> = db
        .query_row(
            "SELECT digest FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![
                format!("seat:{}", request.claim.seat.as_str()),
                request.operation.as_str()
            ],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let (revision, invalidation) = if let Some(digest) = stored {
        if digest != request.payload_hash {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "operation key reused with different payload",
            ));
        }
        // Historical replay is not authority to mutate. Decision validation still
        // rejects this stale context if no exact operation record is found.
        (0, 0)
    } else {
        if let ObligationRef::AcceptCurrent(thread) = &request.obligation {
            let invitation:Option<String>=db.query_row("SELECT i.id FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) ORDER BY i.episode DESC LIMIT 1",
                params![thread.as_str(),request.claim.seat.as_str()],|r|r.get(0)).optional().map_err(store_error)?;
            request.obligation =
                ObligationRef::Invitation(crate::protocol::ids::InvitationId::new(
                    invitation.ok_or_else(|| {
                        api_error(ErrorCode::NotFound, "current invitation missing")
                    })?,
                ));
        }
        let mapping = cooperative_mapping(db, &request.claim, request.check_in_mode)?;
        (mapping.revision, mapping.invalidation_revision)
    };
    Ok(MutationPermit::cooperative(
        request.claim,
        request.operation,
        request.obligation,
        request.payload_hash,
        context.clock().monotonic_now(),
        (revision, invalidation),
        budget.clone(),
    ))
}

pub fn check_in_payload(command: &CheckIn) -> impl serde::Serialize + '_ {
    ("check_in", &command.mode, &command.claim)
}

// Allowed: one deciding mutation: transaction, permit, claim, mode and the obligation it proves.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decide_cooperative(
    tx: &Transaction<'_>,
    at: DecisionInstant,
    instance: &str,
    permit: &mut MutationPermit,
    claim: &crate::protocol::authority::CallerClaim,
    mode: Option<crate::protocol::commands::CheckInMode>,
    operation: &crate::protocol::ids::OperationId,
    obligation: &ObligationRef,
    digest: &[u8; 32],
) -> Result<CooperativeMapping, ApiError> {
    cooperative_instance(tx, instance, claim)?;
    let mapping = cooperative_mapping(tx, claim, mode)?;
    let fence = crate::protocol::authority::CooperativeDecisionFence {
        now: at.monotonic,
        instance: instance.into(),
        seat: claim.seat.clone(),
        target: claim.target.clone(),
        mapping_revision: mapping.revision,
        // The claim's own generation: equal to the seat's, or one behind a
        // `managed_launch` binding the cooperative mapping accepted.
        binding_generation: mapping.claimed_generation,
        invalidation_revision: mapping.invalidation_revision,
        known_invalidated: false,
    };
    let actor = permit
        .consume_cooperative(&fence, operation, obligation, digest)
        .map_err(|e| api_error(ErrorCode::CallerUnverified, e))?;
    if actor != claim {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative permit context differs from command",
        ));
    }
    Ok(mapping)
}

/// A cooperative check-in constructs its bounded offer inside the deciding SQLite
/// transaction. An offer or encoding failure rolls back the anchor and frontier.
pub fn register_available(
    context: &StoreContext,
    conn: &mut Connection,
    command: &CheckIn,
    operator: Option<&OperatorActor>,
    mut permit: MutationPermit,
    budget: &CallBudget,
    build_offer: impl FnOnce(&Transaction<'_>, &SeatId, u64) -> Result<CheckInResult, ApiError>,
) -> Result<CommandResult, ApiError> {
    let claim = &command.claim;
    let digest = schema::canonical_digest(&check_in_payload(command))?;
    // The operator form never shares an idempotency key with the plain one.
    let idempotency_digest = if operator.is_some() {
        schema::canonical_digest(&("operator_check_in", &command.mode, &command.claim))?
    } else {
        digest
    };
    let scope = format!("seat:{}", claim.seat.as_str());
    schema::execute_budgeted_idempotent_transaction(
        context,
        conn,
        budget,
        &scope,
        command.operation.as_str(),
        idempotency_digest,
        |tx| cooperative_instance(tx, &claim.instance, claim),
        |tx| cooperative_mapping(tx, claim, Some(command.mode)).map(|_| ()),
        |tx, at| {
            let mapping = decide_cooperative(
                tx,
                at,
                &claim.instance,
                &mut permit,
                claim,
                Some(command.mode),
                &command.operation,
                &ObligationRef::CheckIn(claim.seat.clone()),
                &digest,
            )?;
            let seat = &claim.seat;
            let (terminal, incarnation) = mapping.reconfirmation_evidence()?;
            let previously_available:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND registered_at IS NOT NULL AND generation=?2 AND host_boot=?3 AND host_epoch=?4 AND target_generation=?5)",
                params![seat.as_str(),mapping.generation as i64,mapping.boot,mapping.epoch as i64,mapping.revision as i64],|r|r.get(0)).map_err(store_error)?;
            if !previously_available {
                schema::ensure_unavailability_episode(tx, seat)?;
            }
            let lifecycle = matches!(
                command.mode,
                crate::protocol::commands::CheckInMode::Lifecycle { .. }
            );
            if lifecycle
                && claim.harness == crate::protocol::authority::Harness::Human
                && let Some(open) = crate::store::queries::open_binding(tx, seat.as_str())?
                && crate::protocol::authority::AGENT_BINDING_PROVENANCES
                    .contains(&open.provenance.as_str())
                && operator.is_none()
            {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    format!(
                        "seat {} is bound to a {} agent ({}); a person's check-in never replaces an agent's binding. Run it in your own shell pane, or override as the local account: `herdr-threads me init --operator`",
                        seat.as_str(),
                        open.harness,
                        if open.provenance == crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE
                        {
                            "launched, not checked in"
                        } else {
                            open.provenance.as_str()
                        }
                    ),
                ));
            }
            let generation = if lifecycle {
                let maximum:i64=tx.query_row("SELECT MAX(?2,COALESCE(MAX(generation),0)) FROM occupant_bindings WHERE seat_id=?1",
                    params![seat.as_str(),mapping.generation as i64],|r|r.get(0)).map_err(store_error)?;
                let next = maximum.checked_add(1).ok_or_else(|| {
                    api_error(
                        ErrorCode::SequenceExhausted,
                        "seat binding generation exhausted",
                    )
                })?;
                let changed = tx
                    .execute(
                        "UPDATE seats SET generation=?1 WHERE id=?2 AND generation=?3",
                        params![next, seat.as_str(), mapping.generation as i64],
                    )
                    .map_err(store_error)?;
                if changed != 1 {
                    return Err(api_error(
                        ErrorCode::Conflict,
                        "lifecycle generation CAS failed",
                    ));
                }
                tx.execute("UPDATE occupant_bindings SET ended_at=?1 WHERE seat_id=?2 AND ended_at IS NULL",params![at.utc.0,seat.as_str()]).map_err(store_error)?;
                tx.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?13,?10,?10,?11,?12)",
                    params![seat.as_str(),next,claim.target.as_str(),mapping.boot,mapping.epoch as i64,mapping.revision as i64,
                        claim.harness.as_str(),claim.native_session.as_str(),claim.execution.as_str(),at.utc.0,terminal,incarnation,claim.harness.cooperative_provenance()]).map_err(store_error)?;
                catch_up::supersede_stale(tx, seat, at.utc)?;
                if let Some(actor) = operator {
                    tx.execute("INSERT INTO allocation_decisions(instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES (?1,?2,?3,'operator_human_override',?4,?5,?6,?7,?8)",
                        params![claim.instance,claim.target.as_str(),seat.as_str(),at.utc.0,mapping.boot,mapping.epoch as i64,mapping.revision as i64,actor.audit_label()]).map_err(store_error)?;
                }
                tx.execute(
                    "DELETE FROM warning_offer WHERE seat_id=?1",
                    [seat.as_str()],
                )
                .map_err(store_error)?;
                tx.execute("UPDATE wake_work SET binding_generation=?1 WHERE seat_id=?2 AND reservation_id IS NULL",params![next,seat.as_str()]).map_err(store_error)?;
                next
            } else {
                // Restoring availability does not replace the occupant or its generation.
                tx.execute("UPDATE occupant_bindings SET registered_at=?1,host_boot=?2,host_epoch=?3,target_generation=?4,observation_provenance=?9,terminal_id=?7,incarnation=?8 WHERE seat_id=?5 AND generation=?6 AND ended_at IS NULL",
                    params![at.utc.0,mapping.boot,mapping.epoch as i64,mapping.revision as i64,seat.as_str(),mapping.generation as i64,terminal,incarnation,claim.harness.cooperative_provenance()]).map_err(store_error)?;
                mapping.generation as i64
            };
            let seq = schema::next_decision_seq(tx, &claim.instance)?;
            if lifecycle && claim.harness == crate::protocol::authority::Harness::Human {
                // One durable cutoff waives all still-pending seat obligations
                // at this human check-in. No receipt is marked ACKed.
                tx.execute("INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES (?1,?2,?3,?4) ON CONFLICT(seat_id) DO UPDATE SET through_decision_seq=MAX(human_receipt_waivers.through_decision_seq,excluded.through_decision_seq),human_generation=excluded.human_generation,decided_at=excluded.decided_at",
                    params![seat.as_str(),seq as i64,generation,at.utc.0]).map_err(store_error)?;
                schema::clear_waived_receipt_conditions_for_seat(tx, seat.as_str(), at.utc)?;
                let (prepared_high_water, physical_high_water): (i64,i64) = tx.query_row("SELECT COALESCE((SELECT MAX(ordinal) FROM prepared_recipients WHERE seat_id=?1),0),COALESCE((SELECT MAX(ordinal) FROM receipts WHERE seat_id=?1),0)",
                    [seat.as_str()], |r| Ok((r.get(0)?,r.get(1)?))).map_err(store_error)?;
                tx.execute("INSERT INTO human_receipt_reconciliation_bounds(seat_id,prepared_high_water,physical_high_water,decision_seq) VALUES (?1,?2,?3,?4) ON CONFLICT(seat_id) DO UPDATE SET prepared_high_water=excluded.prepared_high_water,physical_high_water=excluded.physical_high_water,decision_seq=excluded.decision_seq",
                    params![seat.as_str(),prepared_high_water,physical_high_water,seq as i64]).map_err(store_error)?;
                tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'human_receipt_reconciliation',?2,?3) ON CONFLICT(kind,subject_id) DO UPDATE SET position=0,high_water=excluded.high_water,status='pending',last_error=NULL,completed_at=NULL",
                    params![format!("work:human-receipts:{}",seat.as_str()),seat.as_str(),prepared_high_water.max(physical_high_water)]).map_err(store_error)?;
            }
            if lifecycle || !previously_available {
                tx.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES (?1,?2,?3,?4,?5)",
                    params![seat.as_str(),seq as i64,at.utc.0,generation,claim.harness.cooperative_provenance()]).map_err(store_error)?;
                let anchor = tx.last_insert_rowid();
                let high_water: i64 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(ordinal),0) FROM prepared_recipients WHERE seat_id=?1",
                        [seat.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'receipt_timer_materialization',?2,?3)",params![format!("receipt-timer:{anchor}"),anchor.to_string(),high_water]).map_err(store_error)?;
            }
            schema::clear_open_unavailability_for_seat(tx, seat.as_str(), at.utc)?;
            tx.execute(
                "UPDATE seats SET unavailability_open=0 WHERE id=?1",
                [seat.as_str()],
            )
            .map_err(store_error)?;
            schema::apply_eligibility_transition(tx, &claim.instance, |_| {
                Ok(lifecycle || !previously_available)
            })?;
            let mut offer = build_offer(tx, seat, seq)?;
            if offer.seat != *seat {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "check-in offer seat mismatch",
                ));
            }
            offer.context = claim.clone();
            offer.context.binding_generation = generation as u64;
            offer
                .inbox
                .validate()
                .map_err(|e| api_error(ErrorCode::InvalidBudget, e))?;
            tx.execute("INSERT INTO warning_offer(seat_id,binding_generation,execution_id,offered_through_seq) VALUES (?1,?2,?3,?4) ON CONFLICT(seat_id) DO UPDATE SET binding_generation=excluded.binding_generation,execution_id=excluded.execution_id,offered_through_seq=MAX(warning_offer.offered_through_seq,excluded.offered_through_seq)",
                params![seat.as_str(),generation,claim.execution.as_str(),seq as i64]).map_err(store_error)?;
            // Lifecycle and Current offers alike settle only the notices
            // they carry (wave-2 fix2 root decision (a)).
            super::attention::settle_carried_notices(
                tx,
                seat.as_str(),
                generation,
                claim.execution.as_str(),
                &offer.notices.items,
            )?;
            Ok(CommandResult::CheckedIn(offer))
        },
        present_check_in_result,
    )
}

/// TRUST-POLICY A3 `managed_launch` (ht-5n6): after `launch` observed Herdr's
/// guarded start correlate with its request, open an unregistered occupant
/// binding for the started harness so the seat has the open binding wake
/// discovery requires, before the agent's first check-in. Decided in one
/// transaction against the canonical view (A2): the seat must be resolved
/// onto the launch target with no recovery hold, the effective observation
/// must be current for the instance's host boot and epoch and the seat's
/// target generation, and the launcher's terminal, Herdr incarnation, boot and
/// target generation must equal it. A seat that already has an open binding
/// (an agent or person that checked in, or an earlier launch) is left
/// unchanged and reported with `recorded: false`. The new binding bumps the
/// seat generation by exactly one, carries placeholder session and execution
/// ids no caller claim can match, has no `registered_at` and starts no
/// availability, receipt timer or wake binding generation: it authorizes
/// nothing but a wake prompt to the bound harness.
pub fn record_managed_launch(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    command: &crate::protocol::commands::RecordManagedLaunch,
) -> Result<CommandResult, ApiError> {
    command
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    let target_generation = checked_host_number(command.target_generation)?;
    context.execute_decision(
        conn,
        |tx| {
            type SeatColumns = Option<(String, Option<String>, i64, i64, Option<String>, i64)>;
            let row: SeatColumns = tx
                .query_row(
                    "SELECT s.state,s.target_id,s.generation,s.target_generation,h.host_boot,h.host_epoch FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1 AND s.instance_id=?2",
                    params![command.seat.as_str(), instance],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .optional()
                .map_err(store_error)?;
            let Some((state, target, generation, seat_target_generation, boot, epoch)) = row else {
                return Err(api_error(ErrorCode::NotFound, "launched seat not found"));
            };
            if state != "resolved" || target.as_deref() != Some(command.target.as_str()) {
                return Err(api_error(
                    ErrorCode::TargetUnresolved,
                    "launched seat is no longer resolved onto the launch pane",
                ));
            }
            let held: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE instance_id=?1 AND target_id=?2 AND released_at IS NULL)",
                    params![instance, command.target.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let observation =
                effective::effective_observation(tx, instance, command.target.as_str())?;
            let Some(observation) = observation.filter(|o| {
                !held
                    && o.structural_generation == seat_target_generation
                    && Some(o.host_boot.as_str()) == boot.as_deref()
                    && o.epoch == epoch
            }) else {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "launch pane is held or its observation is not current",
                ));
            };
            let (terminal, incarnation) = binding_evidence(
                observation.terminal_id.as_deref(),
                observation.incarnation.as_deref(),
            )?;
            if terminal != command.terminal.as_str()
                || incarnation != command.incarnation
                || observation.host_boot != command.host_boot.as_str()
                || observation.structural_generation != target_generation
            {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "launch evidence differs from the current pane observation",
                ));
            }
            Ok((
                generation,
                seat_target_generation,
                observation.host_boot.clone(),
                epoch,
                terminal.to_owned(),
                incarnation.to_owned(),
            ))
        },
        |tx, at, (generation, seat_target_generation, boot, epoch, terminal, incarnation)| {
            let open: Option<(i64, String)> = tx
                .query_row(
                    "SELECT generation,observation_provenance FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY ordinal DESC LIMIT 1",
                    [command.seat.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            if let Some((bound_generation, provenance)) = open {
                return Ok(CommandResult::ManagedLaunchRecorded(
                    crate::protocol::results::ManagedLaunchRecord {
                        seat: command.seat.clone(),
                        recorded: false,
                        binding_generation: u64::try_from(bound_generation).map_err(|_| {
                            api_error(ErrorCode::StoreCorrupt, "negative binding generation")
                        })?,
                        provenance,
                    },
                ));
            }
            // Exactly one past the seat generation, so a lifecycle check-in
            // prepared just before this decision is recognized (cooperative
            // mapping) and still replaces the launch binding.
            let next = generation.checked_add(1).ok_or_else(|| {
                api_error(ErrorCode::SequenceExhausted, "seat binding generation exhausted")
            })?;
            let taken: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id=?1 AND generation>=?2)",
                    params![command.seat.as_str(), next],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if taken {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "seat binding history is ahead of the seat generation; the agent's check-in registers it",
                ));
            }
            let changed = tx
                .execute(
                    "UPDATE seats SET generation=?1 WHERE id=?2 AND generation=?3",
                    params![next, command.seat.as_str(), generation],
                )
                .map_err(store_error)?;
            if changed != 1 {
                return Err(api_error(ErrorCode::Conflict, "managed launch generation CAS failed"));
            }
            let placeholder = format!(
                "{}{}",
                crate::protocol::authority::MANAGED_LAUNCH_PLACEHOLDER_PREFIX,
                uuid::Uuid::new_v4()
            );
            tx.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8,?9,?10,NULL,?11,?12)",
                params![command.seat.as_str(),next,command.target.as_str(),boot,epoch,seat_target_generation,
                    command.harness.as_str(),placeholder,crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE,at.utc.0,terminal,incarnation]).map_err(store_error)?;
            Ok(CommandResult::ManagedLaunchRecorded(
                crate::protocol::results::ManagedLaunchRecord {
                    seat: command.seat.clone(),
                    recorded: true,
                    binding_generation: u64::try_from(next).map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "negative seat generation")
                    })?,
                    provenance: crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE.into(),
                },
            ))
        },
    )
}

fn present_check_in_result(
    tx: &Transaction<'_>,
    mut result: CommandResult,
) -> Result<CommandResult, ApiError> {
    if let CommandResult::CheckedIn(offer) = &mut result {
        let claim = &offer.context;
        let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id WHERE s.id=?1 AND s.instance_id=?2 AND s.state!='retired' AND s.target_id=?3 AND s.generation=?4 AND b.generation=s.generation AND b.ended_at IS NULL AND b.target_id=s.target_id AND b.harness=?5 AND b.native_session=?6 AND b.execution_id=?7)",
            params![claim.seat.as_str(),claim.instance,claim.target.as_str(),claim.binding_generation as i64,claim.harness.as_str(),claim.native_session.as_str(),claim.execution.as_str()],|r|r.get(0)).map_err(store_error)?;
        offer.context_disposition = if current {
            crate::protocol::results::CheckInContextDisposition::Current
        } else {
            crate::protocol::results::CheckInContextDisposition::Historical
        };
    }
    Ok(result)
}
