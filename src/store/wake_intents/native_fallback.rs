//! Native-only degradation retains physical queue effects and bounded frontier coverage.
use super::*;
use crate::ports::NativeFallbackOutcome;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct Claim {
    members: usize,
    reservation: String,
    owner: uuid::Uuid,
    ticket: uuid::Uuid,
    coverage: [Option<i64>; 7],
    prior: [Option<i64>; 7],
    outcome: Option<String>,
}
fn frontier(value: [Option<i64>; 7]) -> Result<LogicalAttentionFrontier, ApiError> {
    fn key(
        seq: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Option<LogicalPublicationKey>, ApiError> {
        match (seq, offset) {
            (None, None) => Ok(None),
            (Some(seq), Some(offset)) => Ok(Some(LogicalPublicationKey {
                decision_seq: unsigned(seq)?,
                event_offset: unsigned(offset)?,
            })),
            _ => Err(corrupt()),
        }
    }
    if value[6].is_some_and(|n| n < 0) {
        return Err(corrupt());
    }
    Ok(LogicalAttentionFrontier {
        invitation: key(value[0], value[1])?,
        addressed_receipt: key(value[2], value[3])?,
        actionable_warning: key(value[4], value[5])?,
        lazy: value[6],
    })
}
fn scalars(value: &LogicalAttentionFrontier) -> Result<[Option<i64>; 7], ApiError> {
    fn part(key: Option<LogicalPublicationKey>, offset: bool) -> Result<Option<i64>, ApiError> {
        key.map(|key| {
            signed(if offset {
                key.event_offset
            } else {
                key.decision_seq
            })
        })
        .transpose()
    }
    Ok([
        part(value.invitation, false)?,
        part(value.invitation, true)?,
        part(value.addressed_receipt, false)?,
        part(value.addressed_receipt, true)?,
        part(value.actionable_warning, false)?,
        part(value.actionable_warning, true)?,
        value.lazy,
    ])
}
fn merge(left: &mut LogicalAttentionFrontier, right: &LogicalAttentionFrontier) {
    left.invitation = left.invitation.max(right.invitation);
    left.addressed_receipt = left.addressed_receipt.max(right.addressed_receipt);
    left.actionable_warning = left.actionable_warning.max(right.actionable_warning);
    left.lazy = left.lazy.max(right.lazy);
}
fn decode(bytes: Option<Vec<u8>>) -> Result<Option<Claim>, ApiError> {
    bytes
        .map(|bytes| {
            if bytes.len() > 2048 {
                return Err(corrupt());
            }
            let claim: Claim = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
            frontier(claim.coverage)?;
            frontier(claim.prior)?;
            if claim.members == 0
                || claim.members > MAX_INTENTS as usize
                || claim.reservation.is_empty()
                || claim.reservation.len() > 128
                || claim.owner.is_nil()
                || claim.ticket.is_nil()
                || !matches!(
                    claim.outcome.as_deref(),
                    None | Some("no_effect" | "submitted" | "unknown")
                )
            {
                return Err(corrupt());
            }
            Ok(claim)
        })
        .transpose()
}
fn bytes(claim: &Claim) -> Result<Vec<u8>, ApiError> {
    let value = serde_json::to_vec(claim).map_err(|_| corrupt())?;
    if value.len() > 2048 {
        return Err(corrupt());
    }
    Ok(value)
}
fn collect(
    db: &Connection,
    instance: &str,
    predicate: &str,
    values: &[&str],
    attempts: &mut BTreeSet<String>,
) -> Result<(), ApiError> {
    let sql = format!(
        "SELECT attempt FROM wake_intents WHERE instance_id=?1 AND {predicate} AND {UNRESOLVED} AND native_no_add_exception=0"
    );
    let mut stmt = db.prepare_cached(&sql).map_err(store_error)?;
    let args = std::iter::once(instance).chain(values.iter().copied());
    for row in stmt
        .query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(0))
        .map_err(store_error)?
    {
        attempts.insert(row.map_err(store_error)?);
    }
    Ok(())
}
fn matching_attempts(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
) -> Result<BTreeSet<String>, ApiError> {
    let mut attempts = BTreeSet::new();
    let any: bool = db.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM wake_intents WHERE instance_id=?1 AND {UNRESOLVED} AND native_no_add_exception=0)"),
        [instance], |r| r.get(0)).map_err(store_error)?;
    if !any {
        return Ok(attempts);
    }
    collect(
        db,
        instance,
        "origin_seat=?2",
        &[seat.as_str()],
        &mut attempts,
    )?;
    let registry = builtins();
    let mut stmt=db.prepare_cached(&format!("SELECT DISTINCT b.harness,b.native_session FROM occupant_bindings b INDEXED BY occupant_bindings_open WHERE b.seat_id=?1 AND b.ended_at IS NULL AND EXISTS(SELECT 1 FROM wake_intents WHERE instance_id=?2 AND adapter=b.harness AND {UNRESOLVED} AND native_no_add_exception=0)")).map_err(store_error)?;
    let rows = stmt
        .query_map(params![seat.as_str(), instance], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(store_error)?;
    for row in rows {
        let (adapter, session) = row.map_err(store_error)?;
        let Ok(adapter) = registry.agent(&adapter) else {
            continue;
        };
        let registration = registry.by_id(adapter).map_err(|_| corrupt())?;
        let Some(scope) =
            registration.wake_replay_scope(&crate::protocol::ids::NativeSessionId::new(session))
        else {
            continue;
        };
        collect(
            db,
            instance,
            "adapter=?2 AND replay_scope=?3",
            &[adapter.as_str(), scope.as_str()],
            &mut attempts,
        )?;
    }
    Ok(attempts)
}

struct Allowance {
    rows: Vec<(String, u64, LogicalAttentionFrontier)>,
    coverage: LogicalAttentionFrontier,
}
// Selection is read-only. The same predicate is rechecked by the final claim
// inside its canonical writer decision; neither a candidate nor time is authority.
fn allowance(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    now: crate::protocol::time::UtcMillis,
    current: &LogicalAttentionFrontier,
    reservation: Option<&WakeAttemptId>,
) -> Result<Option<Allowance>, ApiError> {
    let attempts = matching_attempts(db, instance, seat)?;
    if attempts.is_empty() {
        return Ok(None);
    }
    let mut covered = LogicalAttentionFrontier::default();
    let mut rows = Vec::new();
    for attempt in attempts {
        let Some(row) = load(db, instance, &WakeAttemptId::new(&attempt))? else {
            return Err(corrupt());
        };
        if row.logical != crate::ports::WakeIntentLogical::Active
            || !matches!(
                row.phase,
                WakeIntentPhase::Accepted | WakeIntentPhase::Unknown
            )
            || row.cleanup_quarantined
            || !row.helper.as_ref().is_some_and(|h| h.witness.is_some())
        {
            return Ok(None);
        }
        let (armed, stored): (Option<i64>, Option<Vec<u8>>) = db.query_row(
            "SELECT armed_at_utc,native_fallback_json FROM wake_intents WHERE instance_id=?1 AND attempt=?2",
            params![instance,attempt], |q| Ok((q.get(0)?,q.get(1)?))).map_err(store_error)?;
        if !armed.is_some_and(|armed| now.0.saturating_sub(armed) >= 30_000) {
            return Ok(None);
        }
        let old = decode(stored)?;
        if old
            .as_ref()
            .is_some_and(|old| reservation.is_some_and(|r| old.reservation == r.as_str()))
        {
            return Ok(None);
        }
        let prior = old.map_or(Ok(LogicalAttentionFrontier::default()), |old| {
            frontier(old.coverage)
        })?;
        merge(&mut covered, &prior);
        rows.push((attempt, row.revision, prior));
    }
    if !current.advanced_beyond(&covered) {
        return Ok(None);
    }
    merge(&mut covered, current);
    Ok(Some(Allowance {
        rows,
        coverage: covered,
    }))
}
pub(crate) fn due(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    now: crate::protocol::time::UtcMillis,
    current: &LogicalAttentionFrontier,
) -> Result<bool, ApiError> {
    Ok(allowance(db, instance, seat, now, current, None)?.is_some())
}

pub(crate) fn claim(
    store: &SqliteStore,
    r: &WakeReservation,
    ticket: uuid::Uuid,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    if ticket.is_nil() {
        return Err(invalid("invalid native fallback ticket"));
    }
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(&mut writer,budget,|tx| {
        Ok(store.settings.daemon_boot==Some(r.daemon_boot)
            && store.context.clock().monotonic_now()<=r.lease_until
            && wake::validate_reservation(tx,&store.instance,r)?)
    },|tx,at,current| {
        if !current {return Ok(false)}
        let Some(Allowance {rows,coverage}) = allowance(tx,&store.instance,&r.seat,at.utc,
            &r.attention_witness.frontier(),Some(&r.attempt))? else { return Ok(false); };
        let covered=scalars(&coverage)?;
        let members=rows.len();
        for (attempt,revision,prior) in rows {
            let value=bytes(&Claim {members,reservation:r.attempt.as_str().into(),owner:r.daemon_boot,ticket,
                coverage:covered,prior:scalars(&prior)?,outcome:None})?;
            let changed=tx.execute("UPDATE wake_intents SET native_degraded_at_utc=COALESCE(native_degraded_at_utc,?5),native_fallback_json=?4,diagnostic='queue_unreceived',revision=revision+1,updated_at=?5 WHERE instance_id=?1 AND attempt=?2 AND revision=?3",params![store.instance,attempt,signed(revision)?,value,at.utc.0]).map_err(store_error)?;
            if changed!=1 {return Err(corrupt())}
        }
        Ok(true)
    })
}

pub(crate) fn finish(
    store: &SqliteStore,
    r: &WakeReservation,
    ticket: uuid::Uuid,
    outcome: NativeFallbackOutcome,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(&mut writer,budget,|_|Ok(()),|tx,at,()| {
        // Do not re-resolve the target on completion: only the exact durable ticket
        // can complete, even after a binding changes or the external lease expires.
        let mut stmt=tx.prepare_cached("SELECT attempt,native_fallback_json FROM wake_intents WHERE instance_id=?1 AND native_fallback_json IS NOT NULL").map_err(store_error)?;
        let candidates=stmt.query_map([&store.instance],|q|Ok((q.get::<_,String>(0)?,q.get::<_,Vec<u8>>(1)?))).map_err(store_error)?.collect::<Result<Vec<_>,_>>().map_err(store_error)?;
        drop(stmt);
        let mut matched=Vec::new();
        let mut members=None;
        for (attempt,value) in candidates {
            let Some(mut claim)=decode(Some(value))? else {return Err(corrupt())};
            if claim.reservation!=r.attempt.as_str() || claim.owner!=r.daemon_boot || claim.ticket!=ticket || claim.outcome.is_some() {continue}
            if store.settings.daemon_boot!=Some(claim.owner) {return Ok(false)}
            if members.is_some_and(|count|count!=claim.members) {return Err(corrupt())}
            members=Some(claim.members);
            claim.outcome=Some(match outcome {NativeFallbackOutcome::NoEffect=>"no_effect",NativeFallbackOutcome::Submitted=>"submitted",NativeFallbackOutcome::Unknown=>"unknown"}.into());
            if outcome==NativeFallbackOutcome::NoEffect {claim.coverage=claim.prior;}
            matched.push((attempt,bytes(&claim)?));
        }
        if matched.is_empty() || members!=Some(matched.len()) {return Ok(false)}
        for (attempt,value) in matched {
            tx.execute("UPDATE wake_intents SET native_fallback_json=?3,revision=revision+1,updated_at=?4 WHERE instance_id=?1 AND attempt=?2",params![store.instance,attempt,value,at.utc.0]).map_err(store_error)?;
        }
        Ok(true)
    })
}

pub(crate) fn degraded_at_for_seat(db: &Connection, seat: &str) -> Result<Option<i64>, ApiError> {
    use rusqlite::OptionalExtension;
    let instance: Option<String> = db
        .query_row("SELECT instance_id FROM seats WHERE id=?1", [seat], |r| {
            r.get(0)
        })
        .optional()
        .map_err(store_error)?;
    let Some(instance) = instance else {
        return Ok(None);
    };
    let attempts = matching_attempts(db, &instance, &SeatId::new(seat))?;
    let mut latest = None;
    for attempt in attempts {
        let at: Option<i64> = db.query_row("SELECT native_degraded_at_utc FROM wake_intents WHERE instance_id=?1 AND attempt=?2 AND logical_status='active'", params![instance,attempt], |r|r.get(0)).optional().map_err(store_error)?.flatten();
        latest = latest.max(at);
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn component_high_water_keeps_disappeared_publications_and_ordered_offsets() {
        let key = |decision_seq, event_offset| {
            Some(LogicalPublicationKey {
                decision_seq,
                event_offset,
            })
        };
        let mut covered = LogicalAttentionFrontier {
            invitation: key(10, 99),
            lazy: Some(100),
            ..Default::default()
        };
        let newer = LogicalAttentionFrontier {
            invitation: key(11, 0),
            addressed_receipt: key(20, 3),
            ..Default::default()
        };
        assert!(newer.advanced_beyond(&covered));
        merge(&mut covered, &newer);
        assert_eq!(covered.invitation, key(11, 0));
        assert_eq!(covered.lazy, Some(100));
        let old = LogicalAttentionFrontier {
            invitation: key(10, 99),
            lazy: Some(50),
            ..Default::default()
        };
        assert!(!old.advanced_beyond(&covered));
        assert_eq!(frontier(scalars(&covered).unwrap()).unwrap(), covered);
    }
}
