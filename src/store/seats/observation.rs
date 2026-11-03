//! Host observation, snapshot stage, reconciliation transitions, publication-absence closure and baseline holds.

use crate::{
    ports::{
        BindingEvidence, DurableStructuralProof, DurableWorkAdmission, EvidenceKind,
        ExecutionEvidence, GuardedInvalidationTransition, GuardedSeatTransition, HostId,
        HostInvalidationFence, HostInvalidationReason, HostObservation, HostObservationAdmission,
        HostUiState, InvalidationSeatPage, ObservationProvenance, PriorPublishedTarget,
        PublishedSnapshot, ReconciliationAction, ReconciliationOutcome, RestoredPane, SeatState,
        SnapshotCleanupProgress, SnapshotGenerationId, SnapshotHeader, SnapshotSavedSeat,
        SnapshotSeatPage, SnapshotStage, SnapshotStageProgress, SnapshotTargetMatch, StaleReason,
        StructuralOccupancy, UnresolvedReason,
    },
    protocol::{
        authority::CARRIED_BINDING_PROVENANCES,
        ids::{ExecutionId, HostBootId, HostTargetId, SeatId, TerminalId},
        results::{ApiError, ErrorCode},
        time::{CallBudget, UtcMillis},
    },
    store::{
        bindings, catch_up,
        connection::{DecisionInstant, StoreContext, api_error, store_error},
        control, schema,
    },
    sync::facts::seat::RetireReason,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::*;

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

pub(super) fn checked_host_number(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value).map_err(|_| {
        api_error(
            ErrorCode::InvalidRequest,
            "host evidence exceeds SQLite integer range",
        )
    })
}

pub(super) fn snapshot_budget(context: &StoreContext, budget: &CallBudget) -> Result<(), ApiError> {
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

/// Columns of a `snapshot_generations` row, in `load_snapshot_stage`'s SELECT order
/// (the first is the generation's `host_id`).
type SnapshotStageColumns = (i64, String, i64, i64, String, i64, i64, String);

fn load_snapshot_stage(
    conn: &Connection,
    stage: &SnapshotGenerationId,
) -> Result<SnapshotStageColumns, ApiError> {
    conn.query_row(
        "SELECT host_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status FROM snapshot_generations WHERE id=?1",
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
    host: HostId,
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
        |tx, _at, ()| {
            let changed = tx
                .execute(
                    "UPDATE herdr_hosts SET observation_admission_sequence=observation_admission_sequence+1 WHERE host_id=?3 AND node_id=?1 AND observation_admission_sequence<?2",
                    params![instance, i64::MAX, host.get()],
                )
                .map_err(store_error)?;
            if changed != 1 {
                let known: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM herdr_hosts WHERE host_id=?1 AND node_id=?2)",
                        params![host.get(), instance],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                return Err(if known {
                    api_error(ErrorCode::SequenceExhausted, "host observation admission exhausted")
                } else {
                    api_error(ErrorCode::HostUnavailable, "no Herdr server attached")
                });
            }
            let (sequence, active, boot, epoch, lifecycle, invalidation): (
                i64, Option<String>, Option<String>, i64, i64, i64,
            ) = tx
                .query_row(
                    "SELECT observation_admission_sequence,active_snapshot_id,host_boot,host_epoch,lifecycle_revision,invalidation_revision FROM herdr_hosts WHERE host_id=?2 AND node_id=?1",
                    params![instance, host.get()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .map_err(store_error)?;
            Ok(HostObservationAdmission {
                instance: instance.into(),
                host_id: host,
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
                "SELECT host_boot,host_epoch,observation_sequence,active_snapshot_id,lifecycle_revision,invalidation_revision,observation_admission_sequence,observation_decided_sequence FROM herdr_hosts WHERE host_id=?2 AND node_id=?1",
                params![admission.instance(), admission.host_id.get()],
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
                    "SELECT incarnation FROM snapshot_generations WHERE id=?1 AND host_id=?2 AND status='published'",
                    params![active,admission.host_id.get()],
                    |r| r.get(0),
                ).optional().map_err(store_error)?;
                if snapshot_incarnation.as_deref() != Some(proof.incarnation()) {
                    return Ok(false);
                }
            }
            let prior: Option<(String, i64, i64, Option<i64>)> = tx.query_row(
                "SELECT host_boot,epoch,observation_sequence,connection_epoch FROM observed_targets WHERE host_id=?1 AND target_id=?2",
                params![admission.host_id.get(),observation.target.as_str()],
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
                "INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,connection_epoch,incarnation,incarnation_source_kind,observed_at,provenance,terminal_id,occupancy,ui_state,verified_execution,top_level_occupant) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'fresh',?11,?12,?13,?14,?15) ON CONFLICT(host_id,target_id) DO UPDATE SET host_boot=excluded.host_boot,epoch=excluded.epoch,generation=excluded.generation,observation_sequence=excluded.observation_sequence,connection_epoch=excluded.connection_epoch,incarnation=excluded.incarnation,incarnation_source_kind=excluded.incarnation_source_kind,observed_at=excluded.observed_at,provenance='fresh',terminal_id=excluded.terminal_id,occupancy=excluded.occupancy,ui_state=excluded.ui_state,verified_execution=excluded.verified_execution,top_level_occupant=excluded.top_level_occupant",
                params![admission.host_id.get(),observation.target.as_str(),observation.host_boot.as_str(),host_epoch,generation,sequence,
                    proof.as_ref().map(|_| connection_epoch),proof.as_ref().map(DurableStructuralProof::incarnation),
                    proof.as_ref().map(DurableStructuralProof::source_spelling),observation.observed_at_utc.0,
                    observation.terminal.as_ref().map(TerminalId::as_str),occupancy,ui,execution,top_level],
            ).map_err(store_error)?;
            tx.execute(
                "UPDATE herdr_hosts SET observation_decided_sequence=?1 WHERE host_id=?3 AND node_id=?2 AND observation_decided_sequence<?1",
                params![admission_sequence,admission.instance(),admission.host_id.get()],
            ).map_err(store_error)?;
            let current_seat: Option<(String, bool)> = tx.query_row(
                "SELECT s.id, s.target_generation=?3 AND EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=s.id AND b.ended_at IS NULL AND b.registered_at IS NOT NULL AND b.target_id=s.target_id AND b.target_generation=s.target_generation AND b.host_boot=?4 AND b.host_epoch=?5) FROM seats s WHERE s.host_id=?1 AND s.target_id=?2 AND s.state='resolved' AND s.is_local=1",
                params![admission.host_id.get(),observation.target.as_str(),generation,observation.host_boot.as_str(),host_epoch],
                |r| Ok((r.get(0)?,r.get(1)?)),
            ).optional().map_err(store_error)?;
            if let Some((seat, false)) = current_seat {
                schema::ensure_unavailability_episode(tx, &SeatId::new(seat))?;
            }
            schema::apply_eligibility_transition(tx, admission.instance(), |_| Ok(true))?;
            schema::bump_host_lifecycle_revision(tx, admission.host_id)?;
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
                    "SELECT observation_admission_sequence,observation_decided_sequence FROM herdr_hosts WHERE host_id=?2 AND node_id=?1",
                    params![admission.instance(), admission.host_id.get()],
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
                    "UPDATE herdr_hosts SET observation_decided_sequence=?1,invalidation_revision=invalidation_revision+1 WHERE host_id=?4 AND node_id=?2 AND observation_decided_sequence<?1 AND invalidation_revision<?3",
                    params![sequence, admission.instance(), i64::MAX, admission.host_id.get()],
                )
                .map_err(store_error)?;
            if changed != 1 {
                return Err(api_error(ErrorCode::SequenceExhausted, "host invalidation revision exhausted"));
            }
            schema::apply_eligibility_transition(tx, admission.instance(), |_| Ok(true))?;
            let revision: i64 = tx
                .query_row(
                    "SELECT invalidation_revision FROM herdr_hosts WHERE host_id=?2 AND node_id=?1",
                    params![admission.instance(), admission.host_id.get()],
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
        || header.admission.host_id != header.host_id
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
                    "SELECT host_boot,host_epoch,observation_sequence,active_snapshot_id,lifecycle_revision,invalidation_revision,recovery_epoch,observation_admission_sequence,observation_decided_sequence FROM herdr_hosts WHERE host_id=?2 AND node_id=?1",
                    params![&header.instance, header.host_id.get()],
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
                "INSERT INTO snapshot_generations(id,host_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,status,captured_active_id,captured_lifecycle_revision,captured_invalidation_revision,captured_recovery_epoch,admission_sequence,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,'building',?8,?9,?10,?11,?12,?13)",
                params![id.as_str(), header.host_id.get(), header.boot.as_str(), epoch, sequence, header.incarnation, expected, active, lifecycle, invalidation, recovery_epoch, admission_sequence, at.utc.0],
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
                "uncharacterized snapshot target",
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
    // Compared against the stage's captured active publication. Publication
    // uses the comparison only if that publication is still active (its
    // fence refuses otherwise), and published snapshot rows never change, so
    // the result still holds then. Fresh current-target reads are checked at
    // publication, not here.
    let captured_active: Option<String> = tx
        .query_row(
            "SELECT captured_active_id FROM snapshot_generations WHERE id=?1",
            [stage.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let mut slice_matched = captured_active.is_some();
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
        // Equal to the active publication's row in every column the effective
        // view reads except the volatile UI state, observation time and
        // sequence.
        if let Some(active) = captured_active.as_deref().filter(|_| slice_matched) {
            slice_matched = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM snapshot_targets o WHERE o.generation_id=?1 AND o.target_id=?2 AND o.terminal_id IS ?3 AND o.generation=?4 AND o.connection_epoch IS ?5 AND o.incarnation_source_kind IS ?6 AND o.occupancy=?7 AND o.verified_execution IS ?8 AND o.top_level_occupant=?9)",
                    params![active, target.target.as_str(), target.terminal.as_ref().map(|id| id.as_str()), generation, connection_epoch, incarnation_source, occupancy, execution, top_level],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
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
    context.note_snapshot_stage_slice(
        stage.as_str(),
        captured_active.as_deref(),
        offset,
        staged,
        slice_matched,
    );
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
    // Taken before the decision, whose closures may rerun: publication ends
    // the stage's tracking, and a failed one discards the stage.
    let stage_match = context.take_snapshot_stage_match(stage.as_str());
    context.execute_budgeted_decision(
        conn, budget,
        |tx| {
            type RowColumns = Option<(
                i64, String, i64, i64, String, i64, i64, String,
                Option<String>, i64, i64, Option<i64>, i64,
            )>;
            let row: RowColumns = tx
                .query_row(
                    "SELECT host_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_active_id,captured_lifecycle_revision,captured_invalidation_revision,captured_recovery_epoch,admission_sequence FROM snapshot_generations WHERE id=?1",
                    [stage.as_str()],
                    |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?)),
                )
                .optional()
                .map_err(store_error)?;
            let Some((host_id, boot, epoch, sequence, incarnation, expected, staged, status, captured_active, captured_lifecycle, captured_invalidation, captured_recovery_epoch, admission_sequence)) = row else {
                return Err(api_error(ErrorCode::NotFound, "snapshot stage not found"));
            };
            if status != "sealed" || staged != expected {
                return Err(api_error(ErrorCode::StaleHostObservation, "snapshot stage is incomplete"));
            }
            type CurrentColumns = (
                Option<String>, Option<String>, i64, i64, Option<String>, Option<i64>,
                Option<String>, i64, i64, i64, i64, i64, String,
            );
            let current: CurrentColumns = tx
                .query_row(
                    "SELECT active_snapshot_id,host_boot,host_epoch,observation_sequence,recovery_boot,recovery_epoch,recovery_baseline_generation_id,baseline_hold_unclaimed,lifecycle_revision,invalidation_revision,observation_admission_sequence,observation_decided_sequence,node_id FROM herdr_hosts WHERE host_id=?1",
                    [host_id],
                    |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?)),
                )
                .map_err(store_error)?;
            let (active, old_boot, old_epoch, old_sequence, recovery_boot, recovery_epoch, baseline, old_hold, lifecycle, invalidation, issued, decided, instance) = current;
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
            type PriorColumns = Option<(String, i64, i64, Option<i64>)>;
            let prior: PriorColumns = match active.as_deref() {
                Some(id) => tx
                    .query_row(
                        "SELECT incarnation,expected_targets,captured_lifecycle_revision,published_invalidation_revision FROM snapshot_generations WHERE id=?1 AND status='published'",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()
                    .map_err(store_error)?,
                None => None,
            };
            let changed_or_unknown_incarnation = prior
                .as_ref()
                .is_none_or(|(prior, _, _, _)| *prior != incarnation);
            let nonretired = has_nonretired_seat(tx, HostId::from_row(host_id))?;
            let unresolved: bool = tx
                .query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM seats WHERE host_id=?1 AND {LOCAL_SEAT} AND state='unresolved' LIMIT 1)"),
                    [host_id],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let new_recovery = recovery_boot.as_deref() != Some(boot.as_str())
                || recovery_epoch != Some(epoch);
            // SA17 (b): the named host never arms a hold, whatever it holds:
            // an unresolved row or a first publication over live seats must
            // not silence the liveness of its other bindings.
            let named = host_is_named(tx, HostId::from_row(host_id))?;
            let hold = !named
                && (unresolved || changed_or_unknown_incarnation && nonretired
                    || !new_recovery && old_hold != 0);
            let old_baseline = baseline.clone();
            let baseline = match baseline {
                Some(baseline) if !new_recovery => baseline,
                _ => stage.as_str().to_owned(),
            };
            // Send eligibility (`effective_registered_availability`,
            // `carry_pending`) reads the host boot/epoch, the recovery
            // markers and each target's effective observation: the active
            // publication unless invalidated, or a newer fresh current-target
            // read. A publication that repeats the active one changes none of
            // them and needs no bump (shared contract amendment §8). Anything
            // not proven unchanged bumps: no or an invalidated prior
            // publication; a new boot, epoch, incarnation, recovery, baseline
            // or hold; a lifecycle transition since the prior publication;
            // another target count; a target set not proven equal while
            // staging; or a fresh current-target read in this boot/epoch newer
            // than the prior publication (`effective_observation` prefers it
            // over that publication and this one may supersede it, for any
            // target, in the snapshot or not).
            let unchanged = prior.as_ref().is_some_and(
                |(_, prior_expected, prior_lifecycle, prior_invalidation)| {
                    old_boot.as_deref() == Some(boot.as_str())
                        && old_epoch == epoch
                        && !changed_or_unknown_incarnation
                        && !new_recovery
                        && old_baseline.as_deref() == Some(baseline.as_str())
                        && hold == (old_hold != 0)
                        && prior_invalidation.is_some_and(|revision| revision >= invalidation)
                        && *prior_lifecycle == lifecycle
                        && *prior_expected == expected
                        && (expected == 0
                            || stage_match.as_ref().is_some_and(|stage_match| {
                                active.as_deref() == Some(stage_match.captured_active.as_str())
                                    && stage_match.matched == expected
                            }))
                },
            ) && !tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM observed_targets WHERE host_id=?1 AND provenance='fresh' AND host_boot=?2 AND epoch=?3 AND observation_sequence>?4)",
                    params![host_id, boot, epoch, old_sequence],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(store_error)?;
            // A new recovery with no live saved seats and no hold has an
            // empty reconciliation pass. Complete it at publication, before
            // a cold host's first C1 request can observe a lagging marker.
            // Nonempty passes still belong to the reconciliation worker.
            // New recovery already forces the eligibility bump below.
            let empty_recovery_complete = new_recovery && !nonretired && !hold;
            Ok((instance, host_id, boot, epoch, sequence, incarnation, expected, admission_sequence, invalidation, empty_recovery_complete, baseline, hold, !unchanged || named && old_hold != 0, named))
        },
        |tx, at, (instance, host_id, boot, epoch, sequence, incarnation, expected, admission_sequence, invalidation, empty_recovery_complete, baseline, hold, eligibility_may_change, named)| {
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
                "UPDATE herdr_hosts SET host_boot=?1,host_epoch=?2,observation_sequence=?3,recovery_boot=?1,recovery_epoch=?2,active_snapshot_id=?4,recovery_baseline_generation_id=?5,baseline_hold_unclaimed=?6,observation_decided_sequence=?7,reconciled_boot=CASE WHEN ?9 THEN ?1 ELSE reconciled_boot END,reconciled_epoch=CASE WHEN ?9 THEN ?2 ELSE reconciled_epoch END WHERE host_id=?8",
                params![boot,epoch,sequence,stage.as_str(),baseline,hold,admission_sequence,host_id,empty_recovery_complete],
            )
            .map_err(store_error)?;
            if named {
                // SA17: the publication of the named host is its presence
                // read: it releases every hold a former state left open and
                // gives its seats the fixed structural proof the C4 carry
                // needs after a restart.
                release_named_holds(tx, HostId::from_row(host_id), at.utc)?;
                stamp_named_structure(tx, HostId::from_row(host_id), stage, &boot, epoch, &incarnation)?;
            }
            schema::apply_eligibility_transition(tx, &instance, |_| Ok(eligibility_may_change))?;
            Ok(PublishedSnapshot {
                id: stage.clone(),
                instance,
                host_id: HostId::from_row(host_id),
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

/// Whether `host` is the built-in named host (SA17: it never holds).
fn host_is_named(conn: &Connection, host: HostId) -> Result<bool, ApiError> {
    let kind: Option<String> = conn
        .query_row(
            "SELECT kind FROM herdr_hosts WHERE host_id=?1",
            [host.get()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(kind.as_deref() == Some("named"))
}

/// Release the open recovery holds of the named host: it holds nothing.
fn release_named_holds(tx: &Transaction<'_>, host: HostId, at: UtcMillis) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE recovery_holds SET released_at=?1 WHERE host_id=?2 AND released_at IS NULL",
        params![at.0, host.get()],
    )
    .map_err(store_error)?;
    Ok(())
}

/// SA17 (c): stamp the structural proof (the fixed boot and incarnation of the
/// named host, the publication's terminal and connection epoch) on every
/// resolved named seat the publication shows and that has none yet. The
/// planner's `same_incarnation` rule then holds across a daemon restart, and
/// the C4 carry moves the seat's open bindings to the new epoch.
fn stamp_named_structure(
    tx: &Transaction<'_>,
    host: HostId,
    stage: &SnapshotGenerationId,
    boot: &str,
    epoch: i64,
    incarnation: &str,
) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE seats SET structural_terminal_id=t.terminal_id,structural_incarnation=?3,structural_incarnation_kind=t.incarnation_source_kind,structural_host_boot=?4,structural_host_epoch=?5,structural_connection_epoch=t.connection_epoch,structural_observation_sequence=t.observation_sequence \
         FROM snapshot_targets t \
         WHERE t.generation_id=?1 AND t.target_id=seats.target_id AND seats.host_id=?2 AND seats.is_local=1 AND seats.state='resolved' \
           AND seats.structural_terminal_id IS NULL AND t.terminal_id IS NOT NULL AND t.connection_epoch IS NOT NULL \
           AND t.incarnation_source_kind IS NOT NULL AND t.generation=seats.target_generation",
        params![stage.as_str(), host.get(), incarnation, boot, epoch],
    )
    .map_err(store_error)?;
    Ok(())
}

pub(crate) fn has_nonretired_seat(conn: &Connection, host: HostId) -> Result<bool, ApiError> {
    conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=(SELECT node_id FROM herdr_hosts WHERE host_id=?1) AND host_id=?1 AND {LOCAL_SEAT} AND state!='retired' LIMIT 1)"),
        [host.get()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// The still-active publication `stage`; reconciliation reads only the seats of
/// its `host_id`.
fn active_publication(
    conn: &Connection,
    stage: &SnapshotGenerationId,
) -> Result<PublishedSnapshot, ApiError> {
    type RowColumns = Option<(String, i64, String, i64, i64, String, i64, i64, i64)>;
    let row: RowColumns = conn
        .query_row(
            "SELECT h.node_id,h.host_id,g.host_boot,g.epoch,g.observation_sequence,g.incarnation,g.expected_targets,g.published_invalidation_revision,h.invalidation_revision FROM snapshot_generations g JOIN herdr_hosts h ON h.host_id=g.host_id AND h.active_snapshot_id=g.id WHERE g.id=?1 AND g.status='published'",
            [stage.as_str()],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((
        instance,
        host_id,
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
        host_id: HostId::from_row(host_id),
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

pub(super) fn update_structural_proof(
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
    let binding: Option<(Option<String>, Option<String>, String, String)> = latest_binding(
        conn,
        &id,
        "terminal_id,incarnation,execution_id,host_boot",
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let active_binding_execution: Option<String> = conn
        .query_row(
            "SELECT execution_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND registered_at IS NOT NULL ORDER BY ordinal DESC LIMIT 1",
            [&id],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    // C4: the oldest host epoch among the seat's open registered bindings
    // whose provenance is in the carried set (a native binding re-registers
    // instead) and whose host boot and incarnation are the structural proof's.
    // Every such binding is what `carry_binding_forward` moves (ht-54r.3) and
    // what `carry_pending` assumes will be carried, so a newer binding (a
    // one-shot session registered after the epoch change) never hides an
    // older one that still lags.
    let bound_epoch: Option<i64> = match &structural_proof {
        Some(proof) => conn
            .query_row(
                "SELECT MIN(host_epoch) FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND registered_at IS NOT NULL AND observation_provenance IN (?2,?3,?4,?5) AND host_boot=?6 AND incarnation=?7 AND target_id=?8",
                params![
                    &id,
                    CARRIED_BINDING_PROVENANCES[0],
                    CARRIED_BINDING_PROVENANCES[1],
                    CARRIED_BINDING_PROVENANCES[2],
                    CARRIED_BINDING_PROVENANCES[3],
                    proof.host_boot().as_str(),
                    proof.incarnation(),
                    target.as_deref()
                ],
                |r| r.get(0),
            )
            .map_err(store_error)?,
        None => None,
    };
    let bound_epoch = bound_epoch
        .map(|epoch| {
            u64::try_from(epoch)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative binding host epoch"))
        })
        .transpose()?;
    let prior_published_observation = if state == "unresolved"
        && unresolved_reason.as_deref() == Some("host_invalidation")
    {
        let prior: Option<(String, i64, String, i64, String)> = conn.query_row(
            "SELECT g.id,g.epoch,g.host_boot,s.unresolved_prior_binding_generation,g.incarnation FROM seats s JOIN snapshot_generations g ON g.id=s.unresolved_from_generation_id AND g.status='published' AND g.staged_targets=g.expected_targets WHERE s.id=?1 AND g.host_id=s.host_id",
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
        Some(stage) => match snapshot_match(conn, stage, target.as_deref(), terminal)? {
            // The seat's terminal is not shown, but its target is, with no
            // terminal behind it (an hc terminal kept for a restart).
            None if terminal.is_some() => snapshot_match(conn, stage, target.as_deref(), None)?
                .filter(|shown| shown.terminal.is_none()),
            shown => shown,
        },
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
        restored_pane: RestoredPane::None,
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

/// TRUST-POLICY C2 restored-pane retirement (ht-wpq4): how long the daemon must
/// observe a watched pane's closure before the seat retires.
pub const RESTORED_PANE_GRACE_MS: i64 = 10 * 60 * 1000;
/// The most one gap between two absent passes counts toward the grace (the
/// observation lane's backoff cap), so a suspend, clock step or daemon downtime
/// adds at most this much.
pub const ABSENCE_STEP_CAP_MS: i64 = 30_000;
/// Test builds only: overrides [`RESTORED_PANE_GRACE_MS`] in milliseconds.
#[cfg(feature = "test-support")]
pub const RESTORED_PANE_GRACE_ENV: &str = "HT_TEST_RESTORED_PANE_GRACE_MS";

fn restored_pane_grace_ms() -> i64 {
    #[cfg(feature = "test-support")]
    if let Some(ms) = std::env::var(RESTORED_PANE_GRACE_ENV)
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
    {
        return ms.max(0);
    }
    RESTORED_PANE_GRACE_MS
}

/// The restored-pane evidence of one seat against one publication, with what
/// the applier needs to act on it.
struct RestoredPaneFacts {
    evidence: RestoredPane,
    /// The watched (or to-be-watched) pane's terminal and structural generation.
    terminal: Option<String>,
    target_generation: i64,
    /// Observed closure time including this pass's step (Absent / Closed).
    observed_ms: i64,
}

/// TRUST-POLICY C2 restored-pane retirement (ht-wpq4), the single definition
/// read by `saved_seats_page` and re-derived inside the applier's transaction.
/// Only an `unresolved`/`other` local seat of the publication's server with a
/// target qualifies. The evidence is structural and same-incarnation: the
/// watch names a terminal a coherent publication of this boot and incarnation
/// showed at the seat's target while the recovery baseline held it, and the
/// closure is a later coherent publication showing neither that target nor
/// that terminal. A kept claim for any of the seat's top-level sessions, or
/// another seat naming the terminal, keeps it watching.
fn restored_pane_evidence(
    conn: &Connection,
    seat: &SeatId,
    publication: &PublishedSnapshot,
    now: i64,
) -> Result<RestoredPaneFacts, ApiError> {
    let none = RestoredPaneFacts {
        evidence: RestoredPane::None,
        terminal: None,
        target_generation: 0,
        observed_ms: 0,
    };
    let host = publication.host_id;
    let saved: Option<(String, i64)> = conn
        .query_row(
            &format!("SELECT target_id,generation FROM seats WHERE id=?1 AND instance_id=?2 AND host_id=?3 AND {LOCAL_SEAT} AND state='unresolved' AND unresolved_reason='other' AND target_id IS NOT NULL"),
            params![seat.as_str(), publication.instance, host.get()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((target, generation)) = saved else {
        return Ok(none);
    };
    let claimed_elsewhere = |terminal: &str| -> Result<bool, ApiError> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id!=?1 AND host_id=?2 AND structural_terminal_id=?3 AND structural_incarnation=?4) OR EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id!=?1 AND terminal_id=?3 AND incarnation=?4)",
            params![seat.as_str(), host.get(), terminal, publication.incarnation],
            |r| r.get(0),
        )
        .map_err(store_error)
    };
    type WatchColumns = Option<(String, i64, Option<i64>, i64)>;
    let watch: WatchColumns = conn
        .query_row(
            "SELECT terminal_id,target_generation,absent_seen_at,absent_observed_ms FROM restored_pane_watch WHERE seat_id=?1 AND host_id=?2 AND host_boot=?3 AND incarnation=?4 AND target_id=?5 AND seat_generation=?6",
            params![seat.as_str(), host.get(), publication.boot.as_str(), publication.incarnation, target, generation],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((terminal, target_generation, absent_seen_at, observed)) = watch else {
        // No current watch: watch the pane shown at the target, if the current
        // recovery baseline held it (present, never released) and no seat names it.
        let shown: Option<(Option<String>, i64)> = conn
            .query_row(
                "SELECT t.terminal_id,t.generation FROM snapshot_targets t JOIN herdr_hosts h ON h.host_id=?2 JOIN snapshot_generations b ON b.id=h.recovery_baseline_generation_id AND b.status='published' AND b.host_boot=?4 AND b.incarnation=?5 JOIN snapshot_targets bt ON bt.generation_id=b.id AND bt.target_id=t.target_id WHERE t.generation_id=?1 AND t.target_id=?3 AND NOT EXISTS(SELECT 1 FROM recovery_baseline_releases r WHERE r.host_id=?2 AND r.baseline_generation_id=b.id AND r.target_id=t.target_id)",
                params![publication.id.as_str(), host.get(), target, publication.boot.as_str(), publication.incarnation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(store_error)?;
        return Ok(match shown {
            Some((Some(terminal), target_generation))
                if target_generation > 0 && !claimed_elsewhere(&terminal)? =>
            {
                RestoredPaneFacts {
                    evidence: RestoredPane::Observe,
                    terminal: Some(terminal),
                    target_generation,
                    observed_ms: 0,
                }
            }
            _ => none,
        });
    };
    let shown: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM snapshot_targets WHERE generation_id=?1 AND (target_id=?2 OR terminal_id=?3))",
            params![publication.id.as_str(), target, terminal],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    // Any kept row blocks, whatever its age: `kept_continuity::due` prunes
    // expired rows only after a recorded pass, so while a refusal holds the
    // marker back an old row may be a live session's only claim (it never
    // refreshes `received_at`). Failing safe here costs a delay, never a seat.
    let kept_claim: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM kept_continuity_claims k WHERE k.host_id=?1 AND EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=?2 AND b.role='top_level' AND b.agent_id='' AND b.harness=k.harness AND b.native_session=k.native_session))",
            params![host.get(), seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    // The closure counts only while this recovery's reconciliation has
    // completed (its marker names the current recovery boot/epoch), so C1
    // claims, kept ones included, are being decided.
    let reconciled: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM herdr_hosts WHERE host_id=?1 AND reconciled_boot IS NOT NULL AND reconciled_boot=recovery_boot AND reconciled_epoch=recovery_epoch)",
            [host.get()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let evidence = if shown || kept_claim || !reconciled || claimed_elsewhere(&terminal)? {
        RestoredPane::Watching
    } else {
        RestoredPane::Absent
    };
    let observed_ms = if evidence == RestoredPane::Absent {
        let step = absent_seen_at.map_or(0, |seen| {
            now.saturating_sub(seen).clamp(0, ABSENCE_STEP_CAP_MS)
        });
        observed.saturating_add(step)
    } else {
        observed
    };
    Ok(RestoredPaneFacts {
        evidence: if evidence == RestoredPane::Absent && observed_ms >= restored_pane_grace_ms() {
            RestoredPane::Closed
        } else {
            evidence
        },
        terminal: Some(terminal),
        target_generation,
        observed_ms,
    })
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
    let host = publication.host_id;
    let now = context.clock().utc_now().0;
    let high_water = match high_water_ordinal {
        Some(value) => checked_host_number(value)?,
        None => conn
            .query_row(
                &format!(
                    "SELECT COALESCE(MAX(ordinal),0) FROM seats WHERE host_id=?1 AND {LOCAL_SEAT}"
                ),
                [host.get()],
                |r| r.get(0),
            )
            .map_err(store_error)?,
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT ordinal,id,state,unresolved_reason,target_id,generation FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND host_id=?5 AND {LOCAL_SEAT} AND state!='retired' AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4"
        ))
        .map_err(store_error)?;
    let mut rows = stmt
        .query(params![
            publication.instance,
            after,
            high_water,
            i64::from(limit),
            host.get()
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
        if let Some(mut saved) = saved_seat_scalar(
            conn,
            ordinal,
            id,
            state,
            unresolved_reason,
            target,
            generation,
            Some(published),
        )? {
            saved.restored_pane =
                restored_pane_evidence(conn, &saved.seat, &publication, now)?.evidence;
            seats.push(saved);
        }
    }
    let has_more: bool = conn
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND host_id=?4 AND {LOCAL_SEAT} AND state!='retired' AND ordinal>?2 AND ordinal<=?3)"),
            params![publication.instance,last,high_water,host.get()],
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
            "SELECT invalidation_revision,observation_decided_sequence FROM herdr_hosts WHERE host_id=?2 AND node_id=?1",
            params![fence.instance(), fence.admission.host_id.get()],
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
                &format!(
                    "SELECT COALESCE(MAX(ordinal),0) FROM seats WHERE host_id=?1 AND {LOCAL_SEAT}"
                ),
                [fence.admission.host_id.get()],
                |r| r.get(0),
            )
            .map_err(store_error)?,
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT ordinal,id,state,unresolved_reason,target_id,generation FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND host_id=?5 AND {LOCAL_SEAT} AND state!='retired' AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4"
        ))
        .map_err(store_error)?;
    let mut rows = stmt
        .query(params![
            fence.instance(),
            after,
            high_water,
            i64::from(limit),
            fence.admission.host_id.get()
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
            &format!("SELECT EXISTS(SELECT 1 FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND host_id=?4 AND {LOCAL_SEAT} AND state!='retired' AND ordinal>?2 AND ordinal<=?3)"),
            params![fence.instance(),last,high_water,fence.admission.host_id.get()],
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
    context.take_snapshot_stage_match(stage.as_str());
    let started = context.clock().monotonic_now();
    let (host_id, _, _, _, _, _, _, status) = load_snapshot_stage(&tx, stage)?;
    if status == "published" {
        let retained: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM herdr_hosts WHERE host_id=?1 AND (active_snapshot_id=?2 OR recovery_baseline_generation_id=?2)) OR EXISTS(SELECT 1 FROM seats WHERE unresolved_from_generation_id=?2)",
                params![host_id, stage.as_str()],
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

// Allowed: the compare-and-set names every component of the saved seat it fences.
#[allow(clippy::too_many_arguments)]
fn saved_seat_cas(
    tx: &Transaction<'_>,
    instance: &str,
    host: HostId,
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
            &format!("SELECT state,unresolved_reason,generation,target_id,structural_terminal_id FROM seats WHERE id=?1 AND instance_id=?2 AND host_id=?3 AND {LOCAL_SEAT}"),
            params![seat.as_str(), instance, host.get()],
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
    let mut terminal: Option<String> = if invalidated {
        latest_binding(tx, seat.as_str(), "terminal_id", |r| r.get(0))?
    } else {
        tx.query_row(
            "SELECT terminal_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY ordinal DESC LIMIT 1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
    }
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

/// TRUST-POLICY C2: per-host restore-hold release when nothing is left to
/// protect on that host. Predicate: reconciliation of the host's current
/// recovery boot/epoch finished (persisted marker) and no unresolved nonretired
/// seat of the host remains. Clears `baseline_hold_unclaimed` and releases every
/// open `recovery_holds` row of the host in the caller's transaction. Returns
/// whether it lifted.
pub(crate) fn lift_baseline_hold_if_clear(
    tx: &Transaction<'_>,
    instance: &str,
    host: HostId,
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
            "SELECT recovery_boot,recovery_epoch,reconciled_boot,reconciled_epoch,baseline_hold_unclaimed FROM herdr_hosts WHERE host_id=?2 AND node_id=?1",
            params![instance, host.get()],
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
            &format!("SELECT EXISTS(SELECT 1 FROM seats WHERE host_id=?1 AND {LOCAL_SEAT} AND state='unresolved')"),
            [host.get()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let open_holds: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE host_id=?1 AND released_at IS NULL)",
            [host.get()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if unresolved || (flag == 0 && !open_holds) {
        return Ok(false);
    }
    tx.execute(
        "UPDATE herdr_hosts SET baseline_hold_unclaimed=0 WHERE host_id=?1",
        [host.get()],
    )
    .map_err(store_error)?;
    tx.execute(
        "UPDATE recovery_holds SET released_at=?1 WHERE host_id=?2 AND released_at IS NULL",
        params![at.0, host.get()],
    )
    .map_err(store_error)?;
    schema::bump_host_lifecycle_revision(tx, host)?;
    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
    Ok(true)
}

/// Persist that the saved-seat pass of `published` finished with no refusal
/// that could still change a C1 answer or the unresolved set (TRUST-POLICY C2),
/// then run the hold lift in the same transaction. Writes only
/// while `published` is still the active publication of the instance's current
/// recovery boot/epoch. Returns whether `published` was current, so that the
/// marker now names its boot/epoch (false for a stale publication); the
/// marker row is written, and the send-eligibility fence bumped, only when the
/// marker actually moves.
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
                Err(error) if error.code == ErrorCode::CursorStale => return Ok(None),
                Err(error) => return Err(error),
            };
            if current != *published {
                return Ok(None);
            }
            let host = current.host_id;
            let recovery: Option<(Option<String>, Option<i64>)> = tx
                .query_row(
                    "SELECT recovery_boot,recovery_epoch FROM herdr_hosts WHERE host_id=?1",
                    [host.get()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            Ok((recovery == Some((Some(published.boot.as_str().to_owned()), Some(epoch)))).then_some(host))
        },
        |tx, at, current| {
            let Some(host) = current else {
                return Ok(false);
            };
            // `carry_pending` reads the marker: moving it can end a pending
            // carry the planner did not schedule (it plans from the latest
            // open registered binding only), turning a recipient staged
            // without an unavailable warning into one that needs it. A send
            // prepared before the move must not publish that stale staging,
            // so a moving marker bumps the send-eligibility fence. It moves
            // once per recovery boot/epoch; the re-record after every later
            // complete pass changes nothing and stays quiet.
            let moves = tx
                .execute(
                    "UPDATE herdr_hosts SET reconciled_boot=?1,reconciled_epoch=?2 WHERE host_id=?3 AND (reconciled_boot IS NOT ?1 OR reconciled_epoch IS NOT ?2)",
                    params![published.boot.as_str(), epoch, host.get()],
                )
                .map_err(store_error)?
                != 0;
            // A lift bumps both fences itself.
            let lifted = lift_baseline_hold_if_clear(tx, &published.instance, host, at.utc)?;
            schema::apply_eligibility_transition(tx, &published.instance, |_| {
                Ok(moves && !lifted)
            })?;
            Ok(true)
        },
    )
}

fn mark_seat_unresolved(
    tx: &Transaction<'_>,
    at: DecisionInstant,
    instance: &str,
    host: HostId,
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
        "SELECT g.id FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.status='published' AND g.staged_targets=g.expected_targets JOIN snapshot_targets t ON t.generation_id=g.id AND t.target_id=s.target_id WHERE s.id=?1 AND s.instance_id=?2 AND s.host_id=?4 AND s.state='resolved' AND s.generation=?3 AND t.terminal_id IS NOT NULL",
        params![seat.as_str(), instance, generation, host.get()],
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
            "UPDATE seats SET state='unresolved',unresolved_reason=?4,unresolved_from_generation_id=?5,unresolved_prior_binding_generation=CASE WHEN ?5 IS NULL THEN NULL ELSE generation END,generation=generation+1 WHERE id=?1 AND instance_id=?2 AND host_id=?6 AND is_local=1 AND generation=?3 AND state='resolved'",
            params![seat.as_str(),instance,generation,reason_sql,prior_generation,host.get()],
        )
        .map_err(store_error)?;
    if changed != 1 {
        return Ok(ReconciliationOutcome::Stale(StaleReason::new(
            "unresolve_seat_changed",
        )));
    }
    // The binding end is honest (disconnect reason `seat_unresolved`), which is what lets the
    // same session reopen its binding after a rebind (ht-54r.3).
    bindings::end_open_bindings(tx, seat, bindings::DisconnectReason::SeatUnresolved, at.utc)?;
    catch_up::supersede_ended_bindings(tx, seat, at.utc)?;
    tx.execute(
        "DELETE FROM warning_offer WHERE seat_id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    tx.execute(
        "DELETE FROM in_turn_offer WHERE seat_id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    tx.execute(
        "UPDATE wake_work SET reservation_id=NULL,reservation_boot=NULL,binding_generation=NULL WHERE seat_id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    schema::ensure_unavailability_episode(tx, seat)?;
    // SA17 (b): the named host never holds, so one unresolved named row does
    // not silence the liveness of the host's other bindings.
    if !host_is_named(tx, host)? {
        tx.execute(
            "UPDATE herdr_hosts SET baseline_hold_unclaimed=1 WHERE host_id=?1 AND recovery_baseline_generation_id IS NOT NULL",
            [host.get()],
        )
        .map_err(store_error)?;
    }
    schema::bump_host_lifecycle_revision(tx, host)?;
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
                    "SELECT invalidation_revision,observation_decided_sequence,active_snapshot_id FROM herdr_hosts WHERE host_id=?2 AND node_id=?1",
                    params![instance, transition.fence.admission.host_id.get()],
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
                transition.fence.admission.host_id,
                &transition.seat,
                transition.expected_binding_generation,
                transition.expected_target.as_ref(),
                transition.expected_terminal.as_ref(),
                false,
            )
        },
        |tx, at, valid| {
            if !valid {
                return Ok(ReconciliationOutcome::Stale(StaleReason::new("invalidation_fence_changed")));
            }
            mark_seat_unresolved(
                tx,
                at,
                transition.fence.instance(),
                transition.fence.admission.host_id,
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
pub(crate) fn carry_binding_forward(
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
        "UPDATE occupant_bindings SET host_epoch=?1,target_generation=?2 WHERE seat_id=?3 AND ended_at IS NULL AND registered_at IS NOT NULL AND host_boot=?4 AND target_id=?5 AND incarnation=?6 AND observation_provenance IN (?7,?8,?9,?10) AND (host_epoch!=?1 OR target_generation!=?2)",
        params![
            checked_host_number(publication.epoch)?,
            target_generation,
            seat.as_str(),
            publication.boot.as_str(),
            target,
            publication.incarnation,
            CARRIED_BINDING_PROVENANCES[0],
            CARRIED_BINDING_PROVENANCES[1],
            CARRIED_BINDING_PROVENANCES[2],
            CARRIED_BINDING_PROVENANCES[3]
        ],
    ).map_err(store_error)?;
    // A seat holds many bindings (ht-54r.3): every open registered carried binding moves, and
    // the seat's availability anchor is written once (when available), from the most recent. The seat
    // generation is the fence (the caller's compare-and-set); a binding's own is informational.
    if changed == 0 {
        return Ok(false);
    }
    let binding_generation = checked_host_number(generation)?;
    let provenance: String = tx
        .query_row(
            "SELECT observation_provenance FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND registered_at IS NOT NULL AND host_boot=?2 AND host_epoch=?3 AND target_id=?4 AND observation_provenance IN (?5,?6,?7,?8) ORDER BY ordinal DESC LIMIT 1",
            params![
                seat.as_str(),
                publication.boot.as_str(),
                checked_host_number(publication.epoch)?,
                target,
                CARRIED_BINDING_PROVENANCES[0],
                CARRIED_BINDING_PROVENANCES[1],
                CARRIED_BINDING_PROVENANCES[2],
                CARRIED_BINDING_PROVENANCES[3]
            ],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    tx.execute(
        "UPDATE wake_work SET binding_generation=?1 WHERE seat_id=?2 AND reservation_id IS NULL",
        params![binding_generation, seat.as_str()],
    )
    .map_err(store_error)?;
    // C4: the carry moves rows regardless of liveness (a disconnected row resumes on its next
    // reconnect), but availability follows liveness: only a carry that leaves the seat
    // effectively available (a connected carried row) anchors availability, starting receipt
    // timers, and closes the unavailability episode. The seat's target generation moves with
    // the rows first (the caller's later write of the same value is idempotent).
    tx.execute(
        "UPDATE seats SET target_generation=?1 WHERE id=?2 AND generation=?3 AND state='resolved'",
        params![target_generation, seat.as_str(), binding_generation],
    )
    .map_err(store_error)?;
    if schema::effective_registered_availability(tx, seat.as_str(), Some(instance))?.is_none() {
        return Ok(true);
    }
    let seq = schema::next_decision_seq(tx, instance)?;
    anchor_seat_availability(tx, seat, seq, at, binding_generation, &provenance)?;
    schema::clear_open_unavailability_for_seat(tx, seat.as_str(), at)?;
    tx.execute(
        "UPDATE seats SET unavailability_open=0 WHERE id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    Ok(true)
}

/// What reconciliation reads of one open binding's placement.
struct OpenPlacement {
    host_boot: String,
    incarnation: Option<String>,
    execution: String,
    terminal: Option<String>,
    registered: bool,
}

/// The seat's latest binding row: the newest open row, else the most
/// recently ended. Ordinal alone is not recency: a binding reopened after a
/// rebind (`bindings::resolve`) keeps its original ordinal, so rows that ended
/// before the reopen can sort above it (ht-kaw9). Two statements, so the
/// precedence is explicit: the open row through the `occupant_bindings_open`
/// index, and only for a seat with none, its ended history. `columns` selects
/// from `occupant_bindings`.
pub(super) fn latest_binding<T>(
    conn: &Connection,
    seat: &str,
    columns: &str,
    map: impl Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Option<T>, ApiError> {
    let open = conn
        .query_row(
            &format!("SELECT {columns} FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY ordinal DESC LIMIT 1"),
            [seat],
            &map,
        )
        .optional()
        .map_err(store_error)?;
    if open.is_some() {
        return Ok(open);
    }
    conn.query_row(
        &format!("SELECT {columns} FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NOT NULL ORDER BY ended_at DESC, ordinal DESC LIMIT 1"),
        [seat],
        map,
    )
    .optional()
    .map_err(store_error)
}

/// The seat's open bindings' placements, most recent first. A seat holds many (ht-54r.3).
fn open_binding_placements(
    tx: &Transaction<'_>,
    seat: &SeatId,
) -> Result<Vec<OpenPlacement>, ApiError> {
    let mut statement = tx
        .prepare("SELECT host_boot,incarnation,execution_id,terminal_id,registered_at IS NOT NULL FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY ordinal DESC")
        .map_err(store_error)?;
    let rows = statement
        .query_map([seat.as_str()], |r| {
            Ok(OpenPlacement {
                host_boot: r.get(0)?,
                incarnation: r.get(1)?,
                execution: r.get(2)?,
                terminal: r.get(3)?,
                registered: r.get(4)?,
            })
        })
        .map_err(store_error)?;
    rows.collect::<Result<_, _>>().map_err(store_error)
}

/// A binding just became available: its availability anchor and the
/// receipt-timer job for every recipient row already staged for the seat
/// (same rows as a fresh registration writes).
pub(super) fn anchor_seat_availability(
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
                Err(error) if error.code == ErrorCode::CursorStale => return Ok(Err(StaleReason::PUBLICATION_SUPERSEDED)),
                Err(error) => return Err(error),
            };
            if publication != transition.publication {
                return Ok(Err(StaleReason::PUBLICATION_CHANGED));
            }
            let host = publication.host_id;
            // The plan keeps an already-unresolved seat unresolved when this
            // publication cannot reconfirm it (for example its agent exited
            // during the outage). That is a no-op, not a stale plan: treating
            // it as Stale aborted the whole page with CursorStale on every
            // turn, so one unrecoverable seat blocked recovery of every later
            // seat. The seat keeps its reason, so later evidence may still
            // reconfirm a host-invalidated seat.
            // TRUST-POLICY C2 restored-pane rule: the evidence is re-derived
            // at commit (`restored_pane_evidence`); here only the seat CAS.
            if matches!(
                transition.action,
                ReconciliationAction::WatchRestoredPane
                    | ReconciliationAction::NoteRestoredPaneAbsent
                    | ReconciliationAction::RetireRestoredPane { .. }
            ) {
                let current: bool = tx.query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2 AND host_id=?5 AND {LOCAL_SEAT} AND state='unresolved' AND unresolved_reason='other' AND generation=?3 AND target_id IS ?4)"),
                    params![transition.seat.as_str(),publication.instance,checked_host_number(transition.expected_binding_generation)?,transition.expected_target.as_ref().map(HostTargetId::as_str),host.get()],
                    |r| r.get(0),
                ).map_err(store_error)?;
                if !current
                    || matches!(&transition.action, ReconciliationAction::RetireRestoredPane { absent_target } if transition.expected_target.as_ref() != Some(absent_target))
                {
                    return Ok(Err(StaleReason::new("restored_pane_seat_changed")));
                }
                return Ok(Ok((0, None, false, host)));
            }
            if matches!(
                transition.action,
                ReconciliationAction::MarkUnresolved | ReconciliationAction::UnresolveAwaitingTerminal
            ) {
                let already_unresolved: bool = tx.query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2 AND host_id=?5 AND {LOCAL_SEAT} AND state='unresolved' AND generation=?3 AND target_id IS ?4)"),
                    params![transition.seat.as_str(),publication.instance,checked_host_number(transition.expected_binding_generation)?,transition.expected_target.as_ref().map(HostTargetId::as_str),host.get()],
                    |r| r.get(0),
                ).map_err(store_error)?;
                if already_unresolved {
                    return Ok(Ok((0, None, true, host)));
                }
            }
            if !saved_seat_cas(
                    tx,
                    &publication.instance,
                    host,
                    &transition.seat,
                    transition.expected_binding_generation,
                    transition.expected_target.as_ref(),
                    transition.expected_terminal.as_ref(),
                    matches!(transition.action, ReconciliationAction::ReconfirmStructure { .. } | ReconciliationAction::BeginRetirement { .. }),
                )?
            {
                return Ok(Err(StaleReason::new("seat_cas_failed")));
            }
            let generation = match &transition.action {
                ReconciliationAction::MarkUnresolved
                | ReconciliationAction::UnresolveAwaitingTerminal => 0,
                ReconciliationAction::WatchRestoredPane
                | ReconciliationAction::NoteRestoredPaneAbsent
                | ReconciliationAction::RetireRestoredPane { .. } => {
                    unreachable!("restored-pane actions return above")
                }
                ReconciliationAction::ReconfirmStructure { target, terminal }
                | ReconciliationAction::CarryForward { target, terminal } => {
                    if matches!(transition.action, ReconciliationAction::CarryForward { .. }) {
                        let resolved: bool = tx.query_row(
                            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND state='resolved')",
                            [transition.seat.as_str()], |r| r.get(0),
                        ).map_err(store_error)?;
                        if !resolved { return Ok(Err(StaleReason::new("carry_seat_not_resolved"))); }
                    }
                    // Same terminal, same verified boot and incarnation as the
                    // evidence on the seat's latest binding; occupancy and
                    // execution are not consulted (the production adapter
                    // reports both Unknown).
                    if transition.expected_terminal.as_ref() != Some(terminal) {
                        return Ok(Err(StaleReason::new("structure_terminal_changed")));
                    }
                    let Some(observed) = snapshot_match(tx, &publication.id, Some(target.as_str()), Some(terminal.as_str()))? else {
                        return Ok(Err(StaleReason::new("structure_target_absent")));
                    };
                    if observed.target != *target
                        || observed.terminal.as_ref() != Some(terminal)
                        || observed.structural_generation == 0
                        || observed.observation_sequence == 0
                        || observed.observation_sequence > publication.observation_sequence
                        || publication_structural_proof(&publication, &observed)?.is_none()
                    {
                        return Ok(Err(StaleReason::new("structure_snapshot_mismatch")));
                    }
                    let bound: Option<(Option<String>, String, Option<String>)> = latest_binding(
                        tx, transition.seat.as_str(), "terminal_id,host_boot,incarnation",
                        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
                    )?;
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
                        return Ok(Err(StaleReason::new("structure_binding_evidence_mismatch")));
                    }
                    let owned: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM seats WHERE host_id=?1 AND target_id=?2 AND state='resolved' AND id!=?3)",
                        params![host.get(),target.as_str(),transition.seat.as_str()],
                        |r| r.get(0),
                    ).map_err(store_error)?;
                    if owned { return Ok(Err(StaleReason::new("structure_target_owned"))); }
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
                        return Ok(Err(StaleReason::new("move_target_absent")));
                    };
                    if observed.target != *target
                        || observed.terminal.as_ref() != Some(terminal)
                        || observed.structural_generation == 0
                        || observed.observation_sequence == 0
                        || observed.observation_sequence > publication.observation_sequence
                    {
                        return Ok(Err(StaleReason::new("move_snapshot_mismatch")));
                    }
                    if publication_structural_proof(&publication, &observed)?.is_none() {
                        return Ok(Err(StaleReason::new("move_structural_proof_missing")));
                    }
                    let owned: bool = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM seats WHERE host_id=?1 AND target_id=?2 AND state='resolved' AND id!=?3)",
                            params![host.get(),target.as_str(),transition.seat.as_str()],
                            |r| r.get(0),
                        )
                        .map_err(store_error)?;
                    if owned {
                        return Ok(Err(StaleReason::new("move_target_owned")));
                    }
                    // A seat holds many open bindings (ht-54r.3), all in one pane: the evidence
                    // is the most recent one's, and a verified execution may be any of theirs.
                    let open_placements = open_binding_placements(tx, &transition.seat)?;
                    let bound = open_placements
                        .first()
                        .map(|p| (p.host_boot.clone(), p.incarnation.clone(), p.execution.clone()));
                    let structural_matches = structural_continuity_matches(
                        tx, &transition.seat, &publication,
                        transition.expected_target.as_ref(), Some(terminal),
                    )?;
                    if structural_matches == Some(false)
                        || structural_matches.is_none() && !bound.as_ref().is_some_and(|(boot, incarnation, _)| {
                            boot == publication.boot.as_str()
                                && incarnation.as_deref() == Some(&publication.incarnation)
                        })
                    { return Ok(Err(StaleReason::new("move_continuity_mismatch"))); }
                    if bound.is_some() {
                        match observed.verified_execution.as_ref() {
                            // A verified execution must be the bound one: a
                            // different one is a replacement, never a move.
                            Some(execution) => {
                                if observed.occupancy != StructuralOccupancy::Occupied
                                    || !observed.top_level_occupant
                                    || !open_placements.iter().any(|p| p.execution == execution.as_str())
                                {
                                    return Ok(Err(StaleReason::new("move_execution_mismatch")));
                                }
                            }
                            // Unknown execution never unseats a binding (D2):
                            // the same terminal in the same verified server
                            // incarnation (checked above) carries the seat,
                            // its generation and its binding structurally to
                            // the new address. A registered binding whose
                            // occupant the snapshot shows absent is not carried.
                            None => {
                                let registered = open_placements.iter().any(|p| p.registered);
                                let same_terminal = structural_matches == Some(true)
                                    || open_placements.iter().any(|p| p.terminal.as_deref() == Some(terminal.as_str()));
                                if !same_terminal || registered && observed.shows_occupant_absent() {
                                    return Ok(Err(StaleReason::new("move_occupant_absent")));
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
                        { return Ok(Err(StaleReason::new("move_historical_execution_mismatch"))); }
                    }
                    checked_host_number(observed.structural_generation)?
                }
                ReconciliationAction::BeginRetirement { absent_target } => {
                    if transition.expected_target.as_ref() != Some(absent_target) {
                        return Ok(Err(StaleReason::new("retire_target_changed")));
                    }
                    let state: String = tx.query_row("SELECT state FROM seats WHERE id=?1", [transition.seat.as_str()], |r| r.get(0)).map_err(store_error)?;
                    let bound = if state == "resolved" {
                        match structural_continuity_matches(
                            tx, &transition.seat, &publication,
                            transition.expected_target.as_ref(), transition.expected_terminal.as_ref(),
                        )? {
                            Some(true) => Some((publication.boot.as_str().to_owned(), Some(publication.incarnation.clone()))),
                            Some(false) => return Ok(Err(StaleReason::new("retire_continuity_mismatch"))),
                            None => open_binding_placements(tx, &transition.seat)?
                                .first()
                                .map(|p| (p.host_boot.clone(), p.incarnation.clone())),
                        }
                    } else {
                        // Unresolved recovery remains restricted by saved_seat_cas
                        // to host invalidation and its retained published provenance.
                        let historical: Option<(String, Option<String>)> = latest_binding(
                            tx, transition.seat.as_str(), "host_boot,incarnation",
                            |r| Ok((r.get(0)?,r.get(1)?)),
                        )?;
                        match historical {
                            Some(bound) => Some(bound),
                            None => {
                                let prior = tx.query_row(
                                    "SELECT g.host_boot,g.incarnation FROM seats s JOIN snapshot_generations g ON g.id=s.unresolved_from_generation_id AND g.status='published' AND g.staged_targets=g.expected_targets JOIN snapshot_targets t ON t.generation_id=g.id AND t.target_id=s.target_id WHERE s.id=?1 AND s.instance_id=?2 AND s.state='unresolved' AND s.unresolved_reason='host_invalidation' AND s.unresolved_prior_binding_generation=?3 AND t.terminal_id=?4",
                                    params![transition.seat.as_str(),publication.instance,checked_host_number(transition.expected_binding_generation)?.saturating_sub(1),transition.expected_terminal.as_ref().map(TerminalId::as_str)],
                                    |r| Ok((r.get(0)?,Some(r.get(1)?))),
                                ).optional().map_err(store_error)?;
                                match prior {
                                    Some(prior) => Some(prior),
                                    // A seat that never registered: the verified
                                    // structural proof its allocation wrote is
                                    // its only boot and incarnation evidence.
                                    None => tx.query_row(
                                        "SELECT structural_host_boot,structural_incarnation FROM seats WHERE id=?1 AND structural_host_boot IS NOT NULL",
                                        [transition.seat.as_str()],
                                        |r| Ok((r.get(0)?,r.get(1)?)),
                                    ).optional().map_err(store_error)?,
                                }
                            }
                        }
                    };
                    let Some((bound_boot, bound_incarnation)) = bound else { return Ok(Err(StaleReason::new("retire_binding_evidence_missing"))); };
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
                        return Ok(Err(StaleReason::new("retire_pane_still_present")));
                    }
                    let target_generation: i64 = tx
                        .query_row(
                            "SELECT target_generation FROM seats WHERE id=?1",
                            [transition.seat.as_str()],
                            |r| r.get(0),
                        )
                        .map_err(store_error)?;
                    if target_generation <= 0 || transition.expected_binding_generation == i64::MAX as u64 {
                        return Ok(Err(StaleReason::new("retire_target_generation_invalid")));
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
            Ok(Ok((generation, proof, false, host)))
        },
        |tx, at, prepared| {
            let (target_generation, proof, already_unresolved, host) = match prepared {
                Ok(prepared) => prepared,
                Err(reason) => return Ok(ReconciliationOutcome::Stale(reason)),
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
                    host,
                    seat,
                    transition.expected_binding_generation,
                    UnresolvedReason::Other,
                )?,
                ReconciliationAction::UnresolveAwaitingTerminal => mark_seat_unresolved(
                    tx,
                    at,
                    instance,
                    host,
                    seat,
                    transition.expected_binding_generation,
                    UnresolvedReason::HostInvalidation,
                )?,
                ReconciliationAction::ReconfirmStructure { target, .. } => {
                    let changed = tx.execute(
                        "UPDATE seats SET state='resolved',unresolved_reason=NULL,unresolved_from_generation_id=NULL,unresolved_prior_binding_generation=NULL,target_id=?1,target_generation=?2 WHERE id=?3 AND instance_id=?4 AND host_id=?6 AND is_local=1 AND generation=?5 AND state='unresolved' AND unresolved_reason='host_invalidation'",
                        params![target.as_str(),target_generation,seat.as_str(),instance,checked_host_number(transition.expected_binding_generation)?,host.get()],
                    ).map_err(store_error)?;
                    if changed != 1 { return Ok(ReconciliationOutcome::Stale(StaleReason::new("reconfirm_seat_changed"))); }
                    // Nothing to carry: the invalidation that unresolved the
                    // seat ended its bindings and bumped its generation, and no
                    // check-in registers while it is unresolved (ht-0b8).
                    update_structural_proof(tx, seat, proof.as_ref().expect("validated structural proof"))?;
                    schema::bump_host_lifecycle_revision(tx, host)?;
                    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
                    lift_baseline_hold_if_clear(tx, instance, host, at.utc)?;
                    ReconciliationOutcome::Applied
                }
                ReconciliationAction::CarryForward { target, .. } => {
                    let generation = checked_host_number(transition.expected_binding_generation)?;
                    let resolved: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND generation=?2 AND state='resolved')",
                        params![seat.as_str(),generation], |r| r.get(0),
                    ).map_err(store_error)?;
                    if !resolved { return Ok(ReconciliationOutcome::Stale(StaleReason::new("carry_seat_changed"))); }
                    // C4: a carry that moves no binding changes nothing.
                    if !carry_binding_forward(tx, instance, at.utc, seat, transition.expected_binding_generation, &transition.publication, target.as_str(), target_generation)? {
                        return Ok(ReconciliationOutcome::Unchanged);
                    }
                    tx.execute(
                        "UPDATE seats SET target_generation=?1 WHERE id=?2 AND generation=?3 AND state='resolved'",
                        params![target_generation,seat.as_str(),generation],
                    ).map_err(store_error)?;
                    update_structural_proof(tx, seat, proof.as_ref().expect("validated structural proof"))?;
                    schema::bump_host_lifecycle_revision(tx, host)?;
                    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
                    ReconciliationOutcome::Applied
                }
                ReconciliationAction::Move { target, terminal } => {
                    let changed = tx.execute(
                        "UPDATE seats SET target_id=?1,target_generation=?2 WHERE id=?3 AND generation=?4 AND state='resolved'",
                        params![target.as_str(),target_generation,seat.as_str(),checked_host_number(transition.expected_binding_generation)?],
                    ).map_err(store_error)?;
                    if changed != 1 {
                        return Ok(ReconciliationOutcome::Stale(StaleReason::new("move_seat_changed")));
                    }
                    tx.execute(
                        "UPDATE occupant_bindings SET target_id=?1,terminal_id=?2,target_generation=?3 WHERE seat_id=?4 AND ended_at IS NULL",
                        params![target.as_str(),terminal.as_str(),target_generation,seat.as_str()],
                    ).map_err(store_error)?;
                    update_structural_proof(tx, seat, proof.as_ref().expect("validated structural proof"))?;
                    schema::bump_host_lifecycle_revision(tx, host)?;
                    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
                    ReconciliationOutcome::Applied
                }
                ReconciliationAction::BeginRetirement { absent_target } => {
                    let job = control::begin_retirement_authored(
                        tx,
                        context,
                        at,
                        RetireReason::Closed,
                        seat.clone(),
                        instance,
                        transition.publication.boot.as_str(),
                        checked_host_number(transition.publication.epoch)?,
                        absent_target.as_str(),
                        target_generation,
                    )?;
                    lift_baseline_hold_if_clear(tx, instance, host, at.utc)?;
                    ReconciliationOutcome::RetirementStarted(job)
                }
                ReconciliationAction::WatchRestoredPane => {
                    let facts = restored_pane_evidence(tx, seat, &transition.publication, at.utc.0)?;
                    let (RestoredPane::Observe, Some(terminal)) = (facts.evidence, facts.terminal) else {
                        return Ok(ReconciliationOutcome::Stale(StaleReason::new("watch_evidence_missing")));
                    };
                    tx.execute(
                        "INSERT INTO restored_pane_watch(seat_id,host_id,host_boot,incarnation,target_id,terminal_id,target_generation,seat_generation,recorded_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) \
                         ON CONFLICT(seat_id) DO UPDATE SET host_id=excluded.host_id,host_boot=excluded.host_boot,incarnation=excluded.incarnation,target_id=excluded.target_id,terminal_id=excluded.terminal_id,target_generation=excluded.target_generation,seat_generation=excluded.seat_generation,recorded_at=excluded.recorded_at,absent_seen_at=NULL,absent_observed_ms=0",
                        params![seat.as_str(),host.get(),transition.publication.boot.as_str(),transition.publication.incarnation,transition.expected_target.as_ref().map(HostTargetId::as_str),terminal,facts.target_generation,checked_host_number(transition.expected_binding_generation)?,at.utc.0],
                    ).map_err(store_error)?;
                    ReconciliationOutcome::Applied
                }
                ReconciliationAction::NoteRestoredPaneAbsent => {
                    let facts = restored_pane_evidence(tx, seat, &transition.publication, at.utc.0)?;
                    if !matches!(facts.evidence, RestoredPane::Absent | RestoredPane::Closed) {
                        return Ok(ReconciliationOutcome::Stale(StaleReason::new("absent_evidence_missing")));
                    }
                    tx.execute(
                        "UPDATE restored_pane_watch SET absent_seen_at=?1,absent_observed_ms=?2 WHERE seat_id=?3",
                        params![at.utc.0, facts.observed_ms, seat.as_str()],
                    ).map_err(store_error)?;
                    ReconciliationOutcome::Applied
                }
                ReconciliationAction::RetireRestoredPane { absent_target } => {
                    let facts = restored_pane_evidence(tx, seat, &transition.publication, at.utc.0)?;
                    if facts.evidence != RestoredPane::Closed {
                        return Ok(ReconciliationOutcome::Stale(StaleReason::new("retire_restored_evidence_missing")));
                    }
                    let job = control::begin_retirement_authored(
                        tx,
                        context,
                        at,
                        RetireReason::Closed,
                        seat.clone(),
                        instance,
                        transition.publication.boot.as_str(),
                        checked_host_number(transition.publication.epoch)?,
                        absent_target.as_str(),
                        facts.target_generation,
                    )?;
                    tx.execute(
                        "UPDATE retirements SET closure_evidence='restored_pane_closed' WHERE id=?1",
                        [job.id.as_str()],
                    ).map_err(store_error)?;
                    tx.execute("DELETE FROM restored_pane_watch WHERE seat_id=?1", [seat.as_str()])
                        .map_err(store_error)?;
                    lift_baseline_hold_if_clear(tx, instance, host, at.utc)?;
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
    host: HostId,
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
                "SELECT host_boot,host_epoch,observation_sequence,recovery_boot,recovery_epoch FROM herdr_hosts WHERE host_id=?2 AND node_id=?1", params![instance, host.get()],
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
            // The attached host's row exists from attach (never created here).
            let attached = tx.execute("UPDATE herdr_hosts SET host_boot=?2,host_epoch=?3,observation_sequence=?4,recovery_boot=?2,recovery_epoch=CASE WHEN ?5 THEN ?3 ELSE recovery_epoch END WHERE host_id=?6 AND node_id=?1",
                params![instance, snapshot.boot.as_str(), snapshot.epoch as i64, snapshot.observation_sequence as i64, new_baseline, host.get()]).map_err(store_error)?;
            if attached != 1 {
                return Err(api_error(ErrorCode::HostUnavailable, "no Herdr server attached"));
            }
            let unresolved: bool = !host_is_named(tx, host)? && tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM seats WHERE host_id=?1 AND {LOCAL_SEAT} AND state='unresolved')"), [host.get()], |r| r.get(0)).map_err(store_error)?;
            for target in &snapshot.targets {
                let prior:Option<(String,i64,i64,i64,i64,String)>=tx.query_row(
                    "SELECT host_boot,epoch,generation,observation_sequence,observed_at,provenance FROM observed_targets WHERE host_id=?1 AND target_id=?2",
                    params![host.get(),target.target.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
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
                tx.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
                    ON CONFLICT(host_id,target_id) DO UPDATE SET host_boot=excluded.host_boot,epoch=excluded.epoch,generation=excluded.generation,observation_sequence=excluded.observation_sequence,observed_at=excluded.observed_at,provenance=excluded.provenance",
                    params![host.get(),target.target.as_str(),snapshot.boot.as_str(),snapshot.epoch as i64,target.generation as i64,
                        target.observation_sequence as i64,target.observed_at_utc.0,provenance_name(target.provenance)]).map_err(store_error)?;
                if prior_generation.is_some_and(|prior|prior!=target.generation as i64) {
                    tx.execute(&format!("UPDATE seats SET target_generation=?1 WHERE host_id=?2 AND {LOCAL_SEAT} AND target_id=?3 AND state='resolved'"),
                        params![target.generation as i64,host.get(),target.target.as_str()]).map_err(store_error)?;
                }
                let host_changed=old.as_ref().is_some_and(|(boot,epoch,_,_,_)|
                    boot.as_deref()!=Some(snapshot.boot.as_str()) || *epoch!=snapshot.epoch as i64);
                if host_changed || prior_generation.is_some_and(|prior|prior!=target.generation as i64) {
                    let current_seat:Option<String>=tx.query_row(&format!("SELECT id FROM seats WHERE host_id=?1 AND {LOCAL_SEAT} AND target_id=?2 AND state='resolved'"),
                        params![host.get(),target.target.as_str()],|r|r.get(0)).optional().map_err(store_error)?;
                    if let Some(current_seat)=current_seat {
                        schema::ensure_unavailability_episode(tx,&SeatId::new(current_seat))?;
                    }
                }
                let owned: bool = tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM seats WHERE host_id=?1 AND {LOCAL_SEAT} AND target_id=?2 AND state='resolved')"),
                    params![host.get(),target.target.as_str()], |r| r.get(0)).map_err(store_error)?;
                let prior: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM recovery_baseline_targets WHERE host_id=?1 AND target_id=?2 AND baseline_boot=?3)",
                    params![host.get(),target.target.as_str(),snapshot.boot.as_str()], |r| r.get(0)).map_err(store_error)?;
                if !new_baseline && prior { continue; }
                let disposition = if owned { "already_owned" } else if new_baseline && unresolved { "held_for_repair" }
                    else if new_baseline { "unambiguous_unclaimed" } else { "created_after_baseline" };
                tx.execute("INSERT INTO recovery_baseline_targets(host_id,target_id,baseline_boot,baseline_epoch,captured_at,disposition) VALUES (?1,?2,?3,?4,?5,?6) \
                    ON CONFLICT(host_id,target_id) DO UPDATE SET baseline_boot=excluded.baseline_boot,baseline_epoch=excluded.baseline_epoch,captured_at=excluded.captured_at,disposition=excluded.disposition",
                    params![host.get(),target.target.as_str(),snapshot.boot.as_str(),snapshot.epoch as i64,decision.utc.0,disposition]).map_err(store_error)?;
                if disposition == "held_for_repair" {
                    tx.execute("INSERT INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) VALUES (?1,?2,?3,?4,'unresolved saved seat continuity') \
                        ON CONFLICT(host_id,target_id) DO UPDATE SET baseline_boot=excluded.baseline_boot,baseline_epoch=excluded.baseline_epoch,reason=excluded.reason,released_at=NULL",
                        params![host.get(),target.target.as_str(),snapshot.boot.as_str(),snapshot.epoch as i64]).map_err(store_error)?;
                }
            }
            schema::apply_eligibility_transition(tx,instance,|_|Ok(eligibility_changed))?;
            Ok(())
        })
}

#[cfg(test)]
#[path = "../../../tests/store/snapshot_eligibility.rs"]
mod snapshot_eligibility_tests;

#[cfg(test)]
#[path = "../../../tests/store/restored_pane.rs"]
mod restored_pane_tests;

#[cfg(test)]
#[path = "../../../tests/store/named_presence.rs"]
mod named_presence_tests;
