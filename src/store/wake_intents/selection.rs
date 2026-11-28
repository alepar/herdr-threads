//! Pure canonical selection and transport fences before physical target capture.
use super::*;
use crate::{
    harness::wake::{WakeControlTarget, WakeScope},
    ports::{NativeTurnPhase, WakeIntentBlockers, WakeQueueBinding, WakeQueueSelection},
    protocol::{
        authority::LivenessFingerprint,
        ids::{BindingId, ExecutionId, NativeSessionId},
    },
};

struct Binding {
    id: String,
    adapter: String,
    session: String,
    execution: String,
    generation: i64,
    pid: Option<i64>,
    start: Option<i64>,
    boot: Option<String>,
    connected_top: bool,
    registered: bool,
    concurrent: bool,
    auxiliary: bool,
    provenance: String,
}

/// Most open connected top-level bindings the queue census reads; one more refuses to queue.
const CENSUS_LIMIT: usize = 64;

/// The root binding of a seat's connected top-level bindings, with the other sessions of its
/// process. One binding is its own root unless it is a concurrent or auxiliary session (a side
/// session is never a target). Several are rooted only when they all share one harness, one process and
/// one kernel boot and the adapter names exactly one root among them; otherwise `None`.
fn process_root(rows: &[Binding]) -> Option<(&Binding, Vec<NativeSessionId>)> {
    match rows {
        [] => None,
        [only] => (!only.concurrent && !only.auxiliary).then(|| (only, Vec::new())),
        [first, rest @ ..] => {
            let same_process = first.pid.is_some()
                && first.start.is_some()
                && first.boot.is_some()
                && rest.iter().all(|r| {
                    r.adapter == first.adapter
                        && r.pid == first.pid
                        && r.start == first.start
                        && r.boot == first.boot
                });
            if !same_process {
                return None;
            }
            let registry = builtins();
            let registration = registry.by_id(registry.agent(&first.adapter).ok()?).ok()?;
            let sessions: Vec<NativeSessionId> = rows
                .iter()
                .map(|r| NativeSessionId::new(&r.session))
                .collect();
            let candidates: Vec<_> = rows
                .iter()
                .zip(&sessions)
                .map(|(r, s)| crate::harness::adapter::RootCandidate {
                    native_session: s,
                    observation_provenance: &r.provenance,
                    concurrent_session: r.concurrent,
                    auxiliary_session: r.auxiliary,
                })
                .collect();
            let root = registration.wake_process_root(&candidates)?;
            let sides = sessions
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != root)
                .map(|(_, s)| s.clone())
                .collect();
            Some((rows.get(root)?, sides))
        }
    }
}

fn blockers(
    db: &Connection,
    instance: &str,
    predicate: &str,
    values: &[&str],
) -> Result<WakeIntentBlockers, ApiError> {
    let sql = format!(
        "SELECT EXISTS(SELECT 1 FROM wake_intents WHERE instance_id=?1 AND {predicate} AND {UNRESOLVED}),EXISTS(SELECT 1 FROM wake_intents WHERE instance_id=?1 AND {predicate} AND {UNRESOLVED} AND native_no_add_exception=0)"
    );
    let args = std::iter::once(instance).chain(values.iter().copied());
    db.query_row(&sql, rusqlite::params_from_iter(args), |r| {
        Ok(WakeIntentBlockers {
            programmatic: r.get(0)?,
            native: r.get(1)?,
        })
    })
    .map_err(store_error)
}

fn retained_fences(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
) -> Result<WakeIntentBlockers, ApiError> {
    let retained: bool = db
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM wake_intents WHERE instance_id=?1 AND {UNRESOLVED})"
            ),
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let mut blocked = crate::ports::WakeIntentBlockers {
        programmatic: false,
        native: false,
    };
    if retained {
        // The origin fence exists even when no current binding can name a cohort.
        blocked = blockers(db, instance, "origin_seat=?2", &[seat.as_str()])?;
        // Cohort fences are independent of eligibility and binding population. The
        // query budget bounds this streaming census; cardinality never invents effects.
        let registry = builtins();
        {
            let mut statement = db.prepare_cached("SELECT DISTINCT harness,native_session FROM occupant_bindings INDEXED BY occupant_bindings_open WHERE seat_id=?1 AND ended_at IS NULL").map_err(store_error)?;
            let cohorts = statement
                .query_map([seat.as_str()], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(store_error)?;
            for row in cohorts {
                let (adapter, session) = row.map_err(store_error)?;
                let Ok(adapter) = registry.agent(&adapter) else {
                    continue;
                };
                let registration = registry.by_id(adapter).map_err(|_| corrupt())?;
                let Some(cohort) = registration.wake_replay_scope(&NativeSessionId::new(session))
                else {
                    continue;
                };
                let fence = blockers(
                    db,
                    instance,
                    "adapter=?2 AND replay_scope=?3",
                    &[adapter.as_str(), cohort.as_str()],
                )?;
                blocked.programmatic |= fence.programmatic;
                blocked.native |= fence.native;
            }
        }
    }
    Ok(blocked)
}

pub(crate) fn queue_wake_selection(
    store: &SqliteStore,
    reservation: &WakeReservation,
    eligibility_needed: bool,
    budget: &CallBudget,
) -> Result<WakeQueueSelection, ApiError> {
    let db = store.context.open_query(budget.clone())?;
    let tx = db.unchecked_transaction().map_err(store_error)?;
    let blocked = retained_fences(&tx, &store.instance, &reservation.seat)?;
    if !eligibility_needed {
        tx.rollback().map_err(store_error)?;
        store.live_budget(budget)?;
        return Ok(WakeQueueSelection {
            binding: None,
            blockers: blocked,
            programmatic_enabled: false,
        });
    }
    // Per-seat opt-in, read in this transaction so a toggle applies to the next attempt. An
    // unreadable value reads off: a read error must never select the queue.
    if !crate::store::seat_config::programmatic_wake_on(&tx, &reservation.seat) {
        tx.rollback().map_err(store_error)?;
        store.live_budget(budget)?;
        return Ok(WakeQueueSelection {
            binding: None,
            blockers: blocked,
            programmatic_enabled: false,
        });
    }
    let rows = {
        let mut statement = tx.prepare_cached("SELECT binding_id,harness,native_session,execution_id,generation,liveness_pid,liveness_start,liveness_boot,(ended_at IS NULL AND liveness='connected' AND role='top_level'),(agent_id='' AND registered_at IS NOT NULL AND observation_provenance='cooperative_top_level'),concurrent_session,observation_provenance,auxiliary_session FROM occupant_bindings INDEXED BY occupant_bindings_open WHERE seat_id=?1 AND ended_at IS NULL AND liveness='connected' AND role='top_level' ORDER BY ordinal DESC LIMIT 65").map_err(store_error)?;
        statement
            .query_map([reservation.seat.as_str()], |r| {
                Ok(Binding {
                    id: r.get(0)?,
                    adapter: r.get(1)?,
                    session: r.get(2)?,
                    execution: r.get(3)?,
                    generation: r.get(4)?,
                    pid: r.get(5)?,
                    start: r.get(6)?,
                    boot: r.get(7)?,
                    connected_top: r.get(8)?,
                    registered: r.get(9)?,
                    concurrent: r.get(10)?,
                    provenance: r.get(11)?,
                    auxiliary: r.get(12)?,
                })
            })
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)?
    };
    let connected: Vec<Binding> = rows.into_iter().filter(|row| row.connected_top).collect();
    let rooted = if connected.len() > CENSUS_LIMIT {
        None
    } else {
        process_root(&connected)
    };
    let current = store.settings.daemon_boot == Some(reservation.daemon_boot)
        && store.context.clock().monotonic_now() <= reservation.lease_until
        && wake::validate_reservation(&tx, &store.instance, reservation)?;
    let selected = if current {
        rooted
            .map(|(row, sides)| select_binding(store, &tx, reservation, row, sides))
            .transpose()?
            .flatten()
    } else {
        None
    };
    tx.rollback().map_err(store_error)?;
    store.live_budget(budget)?;
    Ok(WakeQueueSelection {
        binding: selected,
        blockers: blocked,
        programmatic_enabled: true,
    })
}

fn select_binding(
    store: &SqliteStore,
    db: &Connection,
    reservation: &WakeReservation,
    row: &Binding,
    side_sessions: Vec<NativeSessionId>,
) -> Result<Option<WakeQueueBinding>, ApiError> {
    if reservation
        .authority
        .targets()
        .iter()
        .any(|target| target.dormant)
    {
        return Ok(None);
    }
    if !row.registered {
        return Ok(None);
    }
    let registry = builtins();
    let (Ok(adapter), Some(pid), Some(start), Some(boot), Ok(generation)) = (
        registry.agent(&row.adapter),
        row.pid.and_then(|v| u32::try_from(v).ok()),
        row.start.and_then(|v| u64::try_from(v).ok()),
        row.boot.as_ref(),
        u64::try_from(row.generation),
    ) else {
        return Ok(None);
    };
    let registration = registry.by_id(adapter).map_err(|_| corrupt())?;
    let scope = WakeScope {
        binding: BindingId::new(&row.id),
        native_session: NativeSessionId::new(&row.session),
        execution: ExecutionId::new(&row.execution),
        generation,
        process: LivenessFingerprint { pid, start },
        kernel_boot: boot.clone(),
    };
    let Some(replay_scope) = registration
        .wake_replay_scope(&scope.native_session)
        .filter(|_| registration.wake_control().is_some())
    else {
        return Ok(None);
    };
    let qualified = super::super::native_turn::load(db, &store.instance, &scope.binding)?
        .is_some_and(|turn| turn.scope == scope && turn.phase == NativeTurnPhase::Ended);
    let preparation = WakeIntentPreparation {
        adapter,
        replay_scope: replay_scope.clone(),
        target: WakeControlTarget {
            scope: scope.clone(),
            descriptor: Vec::new(),
        },
        correlation: reservation.attempt.as_str().into(),
        side_sessions: Vec::new(),
    };
    if !qualified || !canonical(store, db, reservation, &preparation)? {
        return Ok(None);
    }
    Ok(Some(WakeQueueBinding {
        adapter,
        scope,
        replay_scope,
        side_sessions,
    }))
}

#[cfg(test)]
#[path = "../../../tests/store/wake_queue_cost.rs"]
mod tests;
