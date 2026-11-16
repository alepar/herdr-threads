//! Canonical current activity observations; never message or queue receipts.
use super::{SqliteStore, api_error, store_error};
use crate::{
    harness::{adapter::TurnEdge, wake::WakeScope},
    ports::{NativeTurnPhase, NativeTurnState, NativeTurnUpdate},
    protocol::{
        authority::{CallerClaim, CallerRole, LivenessFingerprint},
        commands::TurnStateReport,
        ids::{BindingId, NativeTurnId},
        results::{ApiError, ErrorCode},
        time::CallBudget,
    },
};
use rusqlite::{Connection, OptionalExtension, params};
fn corrupt() -> ApiError {
    api_error(ErrorCode::StoreCorrupt, "invalid native turn state")
}
pub(super) fn load(
    db: &Connection,
    instance: &str,
    binding: &BindingId,
) -> Result<Option<NativeTurnState>, ApiError> {
    let row:Option<(Vec<u8>,String,String,i64)>=db.query_row("SELECT scope_json,turn_id,phase,observation_ordinal FROM native_turn_state WHERE instance_id=?1 AND binding_id=?2",params![instance,binding.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(store_error)?;
    row.map(|(scope, id, phase, ordinal)| {
        Ok(NativeTurnState {
            scope: serde_json::from_slice(&scope).map_err(|_| corrupt())?,
            id: NativeTurnId::parse(id).map_err(|_| corrupt())?,
            phase: match phase.as_str() {
                "started" => NativeTurnPhase::Started,
                "ended" => NativeTurnPhase::Ended,
                "end_first" => NativeTurnPhase::EndFirst,
                "unknown" => NativeTurnPhase::Unknown,
                _ => return Err(corrupt()),
            },
            ordinal: ordinal.try_into().map_err(|_| corrupt())?,
        })
    })
    .transpose()
}
struct Canonical {
    binding: BindingId,
    scope: Option<WakeScope>,
}
fn canonical(
    db: &Connection,
    instance: &str,
    claim: &CallerClaim,
) -> Result<Option<Canonical>, ApiError> {
    if claim.instance != instance || claim.role != CallerRole::TopLevel || claim.is_subagent() {
        return Ok(None);
    }
    let Ok(generation) = i64::try_from(claim.binding_generation) else {
        return Ok(None);
    };
    // A direct projection of the exact open key, with no enrollment, reconnect or sibling choice.
    type Columns = (
        String,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<i64>,
        String,
    );
    let row:Option<Columns>=db.query_row("SELECT b.binding_id,b.liveness_pid,b.liveness_start,b.liveness_boot,k.boot,b.registered_at,b.observation_provenance FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id JOIN herdr_hosts h ON h.host_id=s.host_id LEFT JOIN node_kernel_boot k ON k.id=1 WHERE b.seat_id=?1 AND s.instance_id=?2 AND b.harness=?3 AND b.native_session=?4 AND b.agent_id='' AND b.role='top_level' AND b.execution_id=?5 AND b.generation=?6 AND s.generation=b.generation AND s.state='resolved' AND b.ended_at IS NULL AND b.liveness='connected' AND s.target_id=?7 AND b.target_id=s.target_id AND b.target_generation=s.target_generation AND h.endpoint=?8 AND NOT EXISTS(SELECT 1 FROM recovery_holds hold WHERE hold.host_id=s.host_id AND hold.target_id=s.target_id AND hold.released_at IS NULL)",params![claim.seat.as_str(),instance,claim.harness.as_str(),claim.native_session.as_str(),claim.execution.as_str(),generation,claim.target.as_str(),claim.host.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(store_error)?;
    let Some((binding, pid, start, boot, current_boot, registered, provenance)) = row else {
        return Ok(None);
    };
    let fingerprint = pid.zip(start).and_then(|(pid, start)| {
        Some(LivenessFingerprint {
            pid: pid.try_into().ok()?,
            start: start.try_into().ok()?,
        })
    });
    if claim.liveness.is_some() && claim.liveness != fingerprint {
        return Ok(None);
    }
    let binding = BindingId::parse(binding).map_err(|_| corrupt())?;
    let scope = claim
        .liveness
        .filter(|_| {
            registered.is_some()
                && provenance == crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE
                && boot.is_some()
                && boot == current_boot
        })
        .map(|process| WakeScope {
            binding: binding.clone(),
            native_session: claim.native_session.clone(),
            execution: claim.execution.clone(),
            generation: claim.binding_generation,
            process,
            kernel_boot: boot.expect("checked boot"),
        });
    Ok(Some(Canonical { binding, scope }))
}
fn update(edge: Option<TurnEdge>, changed: bool) -> NativeTurnUpdate {
    NativeTurnUpdate { edge, changed }
}
pub(super) fn record(
    store: &SqliteStore,
    request: &TurnStateReport,
    budget: &CallBudget,
) -> Result<NativeTurnUpdate, ApiError> {
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(&mut writer,budget,|tx|canonical(tx,&store.instance,&request.claim),|tx,_,canonical| {
        let Some(canonical)=canonical else {
            // Queue qualification does not gate cooperative legacy activity.
            let legacy=request.claim.instance==store.instance && request.claim.role==CallerRole::TopLevel && !request.claim.is_subagent();
            return Ok(update(legacy.then_some(request.edge),legacy))
        };
        let old=load(tx,&store.instance,&canonical.binding)?;
        let Some(id)=request.native_turn_id.as_ref().filter(|_|matches!(request.edge,TurnEdge::Busy|TurnEdge::Idle)).filter(|_|canonical.scope.is_some()) else {
            // Missing metadata or queue scope retains the legacy edge without queue qualification.
            if let Some(old)=old.filter(|row|row.phase!=NativeTurnPhase::Unknown) {
                let next=old.ordinal.checked_add(1).and_then(|v|i64::try_from(v).ok()).ok_or_else(corrupt)?;
                tx.execute("UPDATE native_turn_state SET phase='unknown',observation_ordinal=?2 WHERE binding_id=?1",params![canonical.binding.as_str(),next]).map_err(store_error)?;
            }
            return Ok(update(Some(request.edge),true))
        };
        let Some(scope)=canonical.scope else{return Ok(update(None,false))};
        let same=old.as_ref().filter(|row|row.scope==scope);
        if let Some(current)=same {
            if current.id==*id {
                match (request.edge,current.phase) {
                    (TurnEdge::Busy,NativeTurnPhase::Started)=>return Ok(update(Some(TurnEdge::Busy),false)),
                    (TurnEdge::Busy,_)=>return Ok(update(None,false)),
                    (TurnEdge::Idle,NativeTurnPhase::Ended|NativeTurnPhase::EndFirst)=>return Ok(update(Some(TurnEdge::Idle),false)),
                    _=>{},
                }
            } else if request.edge==TurnEdge::Idle && current.phase==NativeTurnPhase::Started {
                return Ok(update(None,false))
            }
        }
        if old.is_none() {
            let count:i64=tx.query_row("SELECT count(*) FROM native_turn_state WHERE instance_id=?1",[&store.instance],|r|r.get(0)).map_err(store_error)?;
            if count>=4096 {return Ok(update(Some(request.edge),false))}
        }
        let next=old.as_ref().map_or(Some(1),|row|row.ordinal.checked_add(1)).and_then(|v|i64::try_from(v).ok()).ok_or_else(corrupt)?;
        let matched=same.is_some_and(|row|row.id==*id && row.phase==NativeTurnPhase::Started);
        let phase=if request.edge==TurnEdge::Busy {"started"} else if matched {"ended"} else {"end_first"};
        let scope=serde_json::to_vec(&scope).map_err(|_|corrupt())?;
        tx.execute("INSERT INTO native_turn_state(binding_id,instance_id,scope_json,turn_id,phase,observation_ordinal) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(binding_id) DO UPDATE SET scope_json=excluded.scope_json,turn_id=excluded.turn_id,phase=excluded.phase,observation_ordinal=excluded.observation_ordinal",params![canonical.binding.as_str(),store.instance,scope,id.as_str(),phase,next]).map_err(store_error)?;
        // A new completed turn proves activity even if its Start report was lost.
        // EndFirst remains ineligible as a matched queue-admission baseline.
        if request.edge==TurnEdge::Busy || phase=="end_first" {
            tx.execute("UPDATE wake_intents SET logical_status='fulfilled',fulfillment_turn=?3,fulfillment_ordinal=?4,revision=revision+1 WHERE instance_id=?1 AND target_json=?2 AND logical_status='active' AND phase IN ('armed','accepted','unknown') AND arm_native_id!=?3 AND arm_native_ordinal<?4",params![store.instance,scope,id.as_str(),next]).map_err(store_error)?;
        }
        Ok(update(Some(request.edge),true))
    })
}
pub(super) fn state(
    store: &SqliteStore,
    binding: &BindingId,
    budget: &CallBudget,
) -> Result<Option<NativeTurnState>, ApiError> {
    let db = store.context.open_query(budget.clone())?;
    load(&db, &store.instance, binding)
}
