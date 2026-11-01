//! Operator repair: `mutate_operator`, rebinding an unresolved seat and operator retirement.

use crate::{
    ports::{HostId, HostKind, OperatorRequest},
    protocol::{
        authority::{DecisionFence, OperatorActor},
        ids::{HostBootId, SeatId, prefix},
        results::{ApiError, CommandResult, ErrorCode},
        time::UtcMillis,
    },
    store::{
        catch_up,
        connection::{StoreContext, api_error, store_error},
        control,
        effective::{self, EffectiveObservationSource},
        schema,
    },
    sync::facts::seat::RetireReason,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::*;

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
        host,
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
            guard.host_id(),
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
            guard.host_id(),
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
            guard.host_id(),
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
            // A remote seat is never rebound or replaced here; refused before any write.
            for named in seat.iter().chain(replace.iter()) {
                require_local_seat(tx, named)?;
            }
            if !observed_matches(
                tx,
                host,
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
                        "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2 AND host_id=?4 AND state='resolved' AND target_id=?3)",
                        params![replace.as_str(), instance, target.as_str(), host.get()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                if !owned || seat.as_ref() == Some(replace) {
                    return Err(api_error(
                        ErrorCode::TargetAlreadyOwned,
                        "replace target is not owned by NEW",
                    ));
                }
            } else if !target_free(tx, host, target.as_str())? {
                return Err(owned_target_refusal(
                    tx,
                    host,
                    target.as_str(),
                    seat.as_ref(),
                )?);
            }
            if let Some(proof) = &structural_proof
                && !structural_proof_matches_current(tx, host, proof)?
            {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "repair structural proof differs from current evidence",
                ));
            }
            if let Some(seat) = &seat {
                let state: Option<(String, i64)> = tx
                    .query_row(
                        &format!("SELECT state,generation FROM seats WHERE id=?1 AND instance_id=?2 AND {LOCAL_SEAT}"),
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
            let fresh = effective::effective_observation(tx, host, target.as_str())?
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
                            host,
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
                        "SELECT h.host_boot,h.host_epoch,s.generation FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id WHERE s.id=?1 AND s.instance_id=?2",
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
                control::begin_retirement_authored(
                    tx,
                    context,
                    at,
                    RetireReason::Operator,
                    replace.clone(),
                    &instance,
                    &boot,
                    epoch,
                    target.as_str(),
                    expected_generation_sql,
                )?;
                tx.execute("INSERT INTO allocation_decisions(host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES (?1,?2,?3,'operator_retire',?4,?5,?6,?7,?8)",
                    params![host.get(),target.as_str(),replace.as_str(),at.utc.0,boot,epoch,expected_generation_sql,actor.audit_label()]).map_err(store_error)?;
            }
            if !target_free(tx, host, target.as_str())? {
                return Err(api_error(
                    ErrorCode::TargetAlreadyOwned,
                    "target became owned",
                ));
            }
            let result = if let Some(seat) = &seat {
                rebind_unresolved_seat(
                    tx,
                    &instance,
                    host,
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
                tx.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,home_node,is_local,host_id) VALUES (?1,?2,'resolved','operator_fresh',?3,1,?4,?5,1,?2,1,?6)",
                    params![seat.as_str(),instance,target.as_str(),expected_generation_sql,at.utc.0,host.get()]).map_err(store_error)?;
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
            tx.execute("INSERT INTO allocation_decisions(host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![host.get(),target.as_str(),result_seat.as_str(),kind,at.utc.0,expected_boot.as_str(),expected_epoch_sql,expected_generation_sql,actor.audit_label()]).map_err(store_error)?;
            release_claimed_target(tx, &instance, host, target.as_str(), at.utc)?;
            lift_baseline_hold_if_clear(tx, &instance, host, at.utc)?;
            Ok(result)
        },
    )?;
    crate::store::operator::validate_result(&typed_command, &result)?;
    Ok(result)
}

/// TRUST-POLICY N (cross-kind rebind refusal): a named seat never rebinds onto a pane or terminal
/// target, and a pane or terminal seat never rebinds onto the named host. A seat whose host kind,
/// or a destination whose kind, is outside [`HostKind::IMPLEMENTED`] is skipped: reattach and
/// rebind never dial a kind this build cannot construct.
fn refuse_cross_kind_rebind(
    tx: &Transaction<'_>,
    seat: &SeatId,
    destination: HostId,
) -> Result<(), ApiError> {
    let kind_of = |sql: &str, key: rusqlite::types::Value| -> Result<Option<HostKind>, ApiError> {
        Ok(tx
            .query_row(sql, [key], |r| r.get::<_, String>(0))
            .optional()
            .map_err(store_error)?
            .map(|stored| HostKind::from_stored(&stored)))
    };
    let source = kind_of(
        "SELECT h.kind FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id WHERE s.id=?1",
        seat.as_str().to_owned().into(),
    )?;
    let target = kind_of(
        "SELECT kind FROM herdr_hosts WHERE host_id=?1",
        destination.get().into(),
    )?;
    let (Some(source), Some(target)) = (source, target) else {
        return Err(api_error(ErrorCode::TargetUnresolved, "source changed"));
    };
    if !source.is_implemented() || !target.is_implemented() {
        return Err(api_error(
            ErrorCode::HostUnavailable,
            "rebind skips a host kind this build does not implement",
        ));
    }
    if (source == HostKind::Named) != (target == HostKind::Named) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "a named seat and a pane or terminal seat never rebind onto each other's host",
        ));
    }
    Ok(())
}

/// The seat-side half of a rebind of an unresolved seat onto `target` on `host`
/// (a seat's host changes only together with its target, here): one
/// generation step, the open binding ended, its warning offer dropped and an
/// unavailability episode open. Operator repair leaves the successor binding to
/// the agent's next check-in; TRUST-POLICY C1 continuity opens it in the same
/// transaction (`decide_continuity`).
pub(super) fn rebind_unresolved_seat(
    tx: &Transaction<'_>,
    instance: &str,
    host: HostId,
    seat: &SeatId,
    target: &str,
    target_generation: i64,
    at: UtcMillis,
) -> Result<(), ApiError> {
    refuse_cross_kind_rebind(tx, seat, host)?;
    let changed = tx.execute("UPDATE seats SET state='resolved',unresolved_reason=NULL,unresolved_from_generation_id=NULL,unresolved_prior_binding_generation=NULL,target_id=?1,host_id=?5,generation=generation+1,target_generation=?2 WHERE id=?3 AND instance_id=?4 AND is_local=1 AND state='unresolved'",
        params![target,target_generation,seat.as_str(),instance,host.get()]).map_err(store_error)?;
    if changed != 1 {
        return Err(api_error(ErrorCode::TargetUnresolved, "source changed"));
    }
    tx.execute(
        "UPDATE occupant_bindings SET ended_at=?1 WHERE seat_id=?2 AND ended_at IS NULL",
        params![at.0, seat.as_str()],
    )
    .map_err(store_error)?;
    catch_up::supersede_ended_bindings(tx, seat, at)?;
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
    schema::ensure_unavailability_episode(tx, seat)?;
    Ok(())
}

/// The target-side half of claiming `target` for a seat: its recovery holds
/// are released, its baseline membership recorded as released and owned, and
/// eligibility and revisions advance. Shared by operator repair and C1.
pub(super) fn release_claimed_target(
    tx: &Transaction<'_>,
    instance: &str,
    host: HostId,
    target: &str,
    at: UtcMillis,
) -> Result<(), ApiError> {
    tx.execute("UPDATE recovery_holds SET released_at=?1 WHERE host_id=?2 AND target_id=?3 AND released_at IS NULL",
        params![at.0,host.get(),target]).map_err(store_error)?;
    let baseline: Option<String> = tx
        .query_row(
            "SELECT recovery_baseline_generation_id FROM herdr_hosts WHERE host_id=?1",
            [host.get()],
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
                "INSERT INTO recovery_baseline_releases(host_id,baseline_generation_id,target_id,decision_seq) VALUES (?1,?2,?3,?4) ON CONFLICT(host_id,baseline_generation_id,target_id) DO NOTHING",
                params![host.get(),baseline,target,checked_host_number(sequence)?],
            )
            .map_err(store_error)?;
        }
    }
    tx.execute("UPDATE recovery_baseline_targets SET disposition='already_owned' WHERE host_id=?1 AND target_id=?2",
        params![host.get(),target]).map_err(store_error)?;
    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
    schema::bump_host_lifecycle_revision(tx, host)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    Ok(())
}

/// Whether the first reconciliation pass of the current recovery epoch has
/// not run yet: seats a Herdr restart will unresolve may still look resolved,
/// and unresolved seats it will rebuild may not exist yet.
pub(super) fn reconciliation_lags(tx: &Transaction<'_>, host: HostId) -> Result<bool, ApiError> {
    Ok(tx
        .query_row(
            "SELECT recovery_boot IS NOT reconciled_boot OR recovery_epoch IS NOT reconciled_epoch FROM herdr_hosts WHERE host_id=?1",
            [host.get()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .unwrap_or(false))
}

/// TRUST-POLICY C3: the refusal for a rebind onto an owned target names both
/// operator resolutions. Falls back to the plain message when the owner is not
/// a live seat of this instance (the observation, not a seat, is then stale).
fn owned_target_refusal(
    tx: &Transaction<'_>,
    host: HostId,
    target: &str,
    old: Option<&SeatId>,
) -> Result<ApiError, ApiError> {
    let owner: Option<String> = tx
        .query_row(
            "SELECT id FROM seats WHERE host_id=?1 AND target_id=?2 AND state='resolved' LIMIT 1",
            params![host.get(), target],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(match (owner, old) {
        (Some(new), Some(old)) => api_error(
            ErrorCode::TargetAlreadyOwned,
            format!(
                "target {target} is owned by live seat {new}; seats are never merged. Abandon the old seat: `herdr-threads human seat retire {old} --operator`, or abandon the new role: `herdr-threads human seat rebind {old} --pane {target} --replace {new} --operator`",
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
                    &format!("SELECT state,generation FROM seats WHERE id=?1 AND instance_id=?2 AND {LOCAL_SEAT}"),
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
            let (boot, epoch, host_id): (Option<String>, i64, i64) = tx
                .query_row(
                    "SELECT h.host_boot,h.host_epoch,h.host_id FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id WHERE s.id=?1",
                    [seat.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(store_error)?;
            let host = HostId::from_row(host_id);
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
            control::begin_retirement_authored(
                tx,
                context,
                at,
                RetireReason::Operator,
                seat.clone(),
                instance,
                &boot,
                epoch,
                &target,
                generation,
            )?;
            tx.execute("INSERT INTO allocation_decisions(host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES (?1,?2,?3,'operator_retire',?4,?5,?6,?7,?8)",
                params![host.get(),target,seat.as_str(),at.utc.0,boot,epoch,generation,actor.audit_label()]).map_err(store_error)?;
            lift_baseline_hold_if_clear(tx, instance, host, at.utc)?;
            Ok(CommandResult::OperatorRetired(seat.clone()))
        },
    )?;
    crate::store::operator::validate_result(&typed, &result)?;
    Ok(result)
}
