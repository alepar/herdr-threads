//! Helper-operation evidence and logical fences, independent of enqueue phase.
use super::*;
use crate::{
    harness::wake::{WakeHelperPresence, WakeHelperPurpose, WakeHelperWitness, WakeReplayScope},
    ports::{WakeHelperOperation, WakeIntentBlockers, WakeIntentLogical},
    protocol::ids::SeatId,
};
use rusqlite::OptionalExtension;

fn helper_bytes(op: &WakeHelperOperation) -> Result<Vec<u8>, ApiError> {
    let bytes = serde_json::to_vec(op).map_err(|_| invalid("invalid helper operation"))?;
    if bytes.len() > 2048 {
        return Err(invalid("helper operation too large"));
    }
    Ok(bytes)
}

pub(crate) fn begin_wake_helper(
    store: &SqliteStore,
    attempt: &WakeAttemptId,
    revision: u64,
    op: &WakeHelperOperation,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    if op.id.is_nil()
        || op.witness.is_some()
        || op.kernel_boot.is_empty()
        || op.kernel_boot.len() > 64
        || store.settings.daemon_boot != Some(op.owner_boot)
    {
        return Err(invalid("invalid helper operation"));
    }
    let bytes = helper_bytes(op)?;
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(
        &mut writer,
        budget,
        |tx| {
            let Some(row) = load(tx, &store.instance, attempt)? else {
                return Ok(false)
            };
            let other: bool = tx.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM wake_intents WHERE instance_id=?1 AND adapter=?2 AND replay_scope=?3 AND attempt!=?4 AND {UNRESOLVED})"),
                params![store.instance, row.preparation.adapter.as_str(), row.preparation.replay_scope.as_str(), attempt.as_str()],
                |r| r.get(0)
            ).map_err(store_error)?;
            if other {
                return Ok(false)
            }
            let boot: Option<String> = tx.query_row(
                "SELECT boot FROM node_kernel_boot WHERE id=1",
                [],
                |r| r.get(0)
            ).optional().map_err(store_error)?;
            Ok(row.revision == revision
                && row.helper.as_ref().is_none_or(|h| h.id != op.id)
                && !row.cleanup_quarantined
                && boot.as_deref() == Some(op.kernel_boot.as_str())
                && match op.purpose {
                    WakeHelperPurpose::Add => row.phase == WakeIntentPhase::Prepared
                        && row.logical == WakeIntentLogical::Active
                        && row.owner_boot == op.owner_boot
                        && row.arm_nonce.is_none()
                        && op.subject.is_none(),
                    WakeHelperPurpose::List => row.phase != WakeIntentPhase::Armed
                        && row.logical != WakeIntentLogical::Active
                        && !row.cleanup_list_attempted
                        && op.subject.is_none()
                        && row.helper.as_ref().is_none_or(|h| h.purpose != WakeHelperPurpose::List),
                    WakeHelperPurpose::Delete => row.phase != WakeIntentPhase::Armed
                        && row.logical != WakeIntentLogical::Active
                        && row.cleanup_list_attempted
                        && op.subject.as_ref().is_some_and(|id|
                            row.cleanup_pending_ids.contains(id)
                                && row.helper.as_ref().is_none_or(|h|
                                    h.purpose != WakeHelperPurpose::Delete
                                        || h.subject.as_ref() != Some(id))),
                })
        },
        |tx, at, valid| {
            if !valid {
                return Ok(false)
            }
            tx.execute(
                "UPDATE wake_intents SET helper_json=?3,cleanup_quarantined=1,native_no_add_exception=0,revision=revision+1,updated_at=?4 WHERE instance_id=?1 AND attempt=?2",
                params![store.instance, attempt.as_str(), bytes, at.utc.0]
            ).map_err(store_error)?;
            Ok(true)
        }
    )
}

pub(crate) fn record_wake_helper_started(
    store: &SqliteStore,
    attempt: &WakeAttemptId,
    op: uuid::Uuid,
    witness: &WakeHelperWitness,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    if witness.process.pid < 2
        || witness.pgid != witness.process.pid
        || witness.kernel_boot.is_empty()
        || witness.kernel_boot.len() > 64
    {
        return Err(invalid("invalid helper witness"));
    }
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(
        &mut writer,
        budget,
        |tx| {
            Ok(load(tx, &store.instance, attempt)?
                .filter(|r| !r.native_no_add_exception
                    && match r.helper.as_ref().map(|h| h.purpose) {
                        Some(WakeHelperPurpose::Add) => r.phase == WakeIntentPhase::Prepared,
                        Some(_) => r.logical != WakeIntentLogical::Active
                            && r.phase != WakeIntentPhase::Armed,
                        None => false,
                    })
                .and_then(|r| r.helper)
                .filter(|h| h.id == op
                    && store.settings.daemon_boot == Some(h.owner_boot)
                    && h.kernel_boot == witness.kernel_boot))
        },
        |tx, at, helper| {
            let Some(mut helper) = helper else {
                return Ok(false)
            };
            if let Some(old) = helper.witness {
                return Ok(old == *witness)
            }
            helper.witness = Some(witness.clone());
            tx.execute(
                "UPDATE wake_intents SET helper_json=?3,revision=revision+1,updated_at=?4 WHERE instance_id=?1 AND attempt=?2",
                params![store.instance, attempt.as_str(), helper_bytes(&helper)?, at.utc.0]
            ).map_err(store_error)?;
            Ok(true)
        }
    )
}

pub(crate) fn record_wake_helper_completion(
    store: &SqliteStore,
    attempt: &WakeAttemptId,
    op: uuid::Uuid,
    confirmed: bool,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(
        &mut writer,
        budget,
        |tx| Ok(load(tx, &store.instance, attempt)?.is_some_and(|r|
            r.helper.is_some_and(|h|
                h.id == op && store.settings.daemon_boot == Some(h.owner_boot)))),
        |tx, at, valid| {
            if !valid {
                return Ok(false)
            }
            // Unconfirmed cleanup cannot revoke positive evidence for this exact op.
            tx.execute(
                "UPDATE wake_intents SET cleanup_quarantined=cleanup_quarantined AND ?3,revision=revision+1,updated_at=?4 WHERE instance_id=?1 AND attempt=?2",
                params![store.instance, attempt.as_str(), !confirmed, at.utc.0]
            ).map_err(store_error)?;
            Ok(true)
        }
    )
}

pub(crate) fn reconcile_wake_helper(
    store: &SqliteStore,
    attempt: &WakeAttemptId,
    op: uuid::Uuid,
    presence: &WakeHelperPresence,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    if !matches!(presence, WakeHelperPresence::Quiescent(_)) {
        return Ok(false);
    }
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(
        &mut writer,
        budget,
        |tx| {
            let Some(row) = load(tx, &store.instance, attempt)?
                .filter(|r| r.cleanup_quarantined) else {
                return Ok(false)
            };
            let Some(helper) = row.helper.filter(|h| h.id == op) else {
                return Ok(false)
            };
            Ok(match presence {
                WakeHelperPresence::Quiescent(crate::harness::wake::WakeHelperQuiescence::BootChanged) => {
                    let boot: Option<String> = tx.query_row(
                        "SELECT boot FROM node_kernel_boot WHERE id=1",
                        [],
                        |r| r.get(0)
                    ).optional().map_err(store_error)?;
                    boot.is_some_and(|b| b != helper.kernel_boot)
                },
                WakeHelperPresence::Quiescent(crate::harness::wake::WakeHelperQuiescence::LeaderEndedAndGroupAbsent) => helper.witness.is_some(),
                _ => false,
            })
        },
        |tx, at, valid| {
            if !valid {
                return Ok(false)
            }
            tx.execute(
                "UPDATE wake_intents SET cleanup_quarantined=0,revision=revision+1,updated_at=?3 WHERE instance_id=?1 AND attempt=?2",
                params![store.instance, attempt.as_str(), at.utc.0]
            ).map_err(store_error)?;
            Ok(true)
        }
    )
}

pub(crate) fn retire_wake_intent_if_scope_ended(
    store: &SqliteStore,
    attempt: &WakeAttemptId,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(
        &mut writer,
        budget,
        |tx| {
            let Some(row) = load(tx, &store.instance, attempt)?
                .filter(|r| r.logical == WakeIntentLogical::Active) else {
                return Ok(false)
            };
            let s = &row.preparation.target.scope;
            // Canonical finality only: disconnected liveness and missing process reads are irrelevant.
            type CanonicalBinding = (
                Option<i64>, String, String, String, i64, String, i64,
                Option<i64>, Option<i64>, Option<String>
            );
            let current: Option<CanonicalBinding> = tx.query_row(
                "SELECT b.ended_at,b.native_session,b.execution_id,b.seat_id,b.generation,s.state,s.generation,b.liveness_pid,b.liveness_start,b.liveness_boot FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id WHERE b.binding_id=?1",
                [s.binding.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?))
            ).optional().map_err(store_error)?;
            let Some((ended, session, execution, seat, generation, state, seat_generation, pid, start, boot)) = current else {
                return Ok(true)
            };
            let mapping_final: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND (host_id!=?2 OR target_id!=?3 OR target_generation!=?4))",
                params![row.origin_seat.as_str(), row.origin_host.get(), row.origin_target.as_str(), signed(row.origin_target_generation)?],
                |r| r.get(0)
            ).map_err(store_error)?;
            if mapping_final {
                return Ok(true)
            }
            Ok(ended.is_some()
                || session != s.native_session.as_str()
                || execution != s.execution.as_str()
                || seat != row.origin_seat.as_str()
                || generation != signed(s.generation)?
                || state == "retired"
                || seat_generation != generation
                || pid.is_some_and(|p| p != i64::from(s.process.pid))
                || start.is_some_and(|p| p != s.process.start as i64)
                || boot.is_some_and(|b| b != s.kernel_boot))
        },
        |tx, at, ended| {
            if !ended {
                return Ok(false)
            }
            tx.execute(
                "UPDATE wake_intents SET logical_status='scope_ended',revision=revision+1,updated_at=?3 WHERE instance_id=?1 AND attempt=?2 AND logical_status='active'",
                params![store.instance, attempt.as_str(), at.utc.0]
            ).map_err(store_error)?;
            Ok(true)
        }
    )
}

pub(crate) fn wake_intent_blockers(
    store: &SqliteStore,
    seat: &SeatId,
    adapter: crate::harness::registry::AgentHarnessId,
    cohort: &WakeReplayScope,
    budget: &CallBudget,
) -> Result<WakeIntentBlockers, ApiError> {
    let db = store.context.open_query(budget.clone())?;
    // Separate indexed seeks avoid an OR across the seat and cohort indexes.
    let mut blocked = WakeIntentBlockers::default();
    for (predicate, values) in [
        ("origin_seat=?2", vec![seat.as_str()]),
        (
            "adapter=?2 AND replay_scope=?3",
            vec![adapter.as_str(), cohort.as_str()],
        ),
    ] {
        let sql = format!(
            "SELECT EXISTS(SELECT 1 FROM wake_intents WHERE instance_id=?1 AND {predicate} AND {UNRESOLVED}),EXISTS(SELECT 1 FROM wake_intents WHERE instance_id=?1 AND {predicate} AND {UNRESOLVED} AND native_no_add_exception=0)"
        );
        let mut args = vec![store.instance.as_str()];
        args.extend(values);
        let (queue, native): (bool, bool) = db
            .query_row(&sql, rusqlite::params_from_iter(args), |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(store_error)?;
        blocked.programmatic |= queue;
        blocked.native |= native;
    }
    Ok(blocked)
}

pub(crate) fn record_wake_cleanup_progress(
    store: &SqliteStore,
    attempt: &WakeAttemptId,
    op: uuid::Uuid,
    listed: bool,
    deleted_id: Option<&str>,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(
        &mut writer,
        budget,
        |tx| Ok(load(tx, &store.instance, attempt)?.filter(|r|
            r.logical != WakeIntentLogical::Active
                && r.helper.as_ref().is_some_and(|h|
                    h.id == op
                        && store.settings.daemon_boot.is_some()
                        && !r.cleanup_quarantined
                        && if listed {
                            h.purpose == WakeHelperPurpose::List && deleted_id.is_none()
                        } else {
                            h.purpose == WakeHelperPurpose::Delete
                                && h.subject.as_deref() == deleted_id
                        }))),
        |tx, at, row| {
            let Some(row) = row else {
                return Ok(false)
            };
            if let Some(id) = deleted_id {
                if !row.owned_ids.iter().any(|v| v == id) {
                    return Ok(false)
                }
                tx.execute(
                    "UPDATE wake_intent_ids SET cleanup_attempted=1 WHERE intent_ordinal=?1 AND queue_id=?2",
                    params![signed(row.ordinal)?, id]
                ).map_err(store_error)?;
            }
            tx.execute(
                "UPDATE wake_intents SET cleanup_list_attempted=cleanup_list_attempted OR ?3,revision=revision+1,updated_at=?4 WHERE instance_id=?1 AND attempt=?2",
                params![store.instance, attempt.as_str(), listed, at.utc.0]
            ).map_err(store_error)?;
            Ok(true)
        }
    )
}
