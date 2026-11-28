//! Retained programmatic-wake effects. Nothing here dispatches or settles delivery.
use super::{SqliteStore, api_error, store_error, wake};
use crate::{
    harness::{
        registry::builtins,
        wake::{WakeControlTarget, WakeReplayScope, WakeScope},
    },
    ports::{
        HostId, LogicalAttentionFrontier, LogicalPublicationKey, PrepareWakeIntent, WakeArmTicket,
        WakeIntentOutcome, WakeIntentPage, WakeIntentPhase, WakeIntentPreparation,
        WakeIntentResult, WakeIntentSnapshot, WakeIntentSummary, WakeReservation,
    },
    protocol::{
        ids::{HostBootId, HostTargetId, SeatId, WakeAttemptId},
        results::{ApiError, ErrorCode},
        time::CallBudget,
    },
};
use rusqlite::{Connection, Row, params, types::FromSql};
use std::collections::BTreeSet;

const MAX_INTENTS: i64 = 4096;
const MAX_IDS: usize = 128;
const UNRESOLVED: &str =
    "((logical_status='active' AND phase!='no_effect') OR cleanup_quarantined=1)";
const COLUMNS: &str = "ordinal,attempt,adapter,replay_scope,origin_seat,origin_host,origin_host_boot,origin_host_epoch,origin_target,origin_target_generation,target_json,descriptor,correlation,invitation_seq,invitation_offset,receipt_seq,receipt_offset,warning_seq,warning_offset,lazy_ordinal,phase,revision,owner_boot,arm_nonce,cleanup_quarantined,diagnostic,ids_complete,logical_status,fulfillment_turn,fulfillment_ordinal,arm_native_id,arm_native_ordinal,helper_json,cleanup_list_attempted,native_no_add_exception,add_helper_json,side_sessions";
fn invalid(detail: &str) -> ApiError {
    api_error(ErrorCode::InvalidRequest, detail)
}
fn corrupt() -> ApiError {
    api_error(ErrorCode::StoreCorrupt, "invalid wake intent record")
}
fn get<T: FromSql>(r: &Row<'_>, n: usize) -> Result<T, ApiError> {
    r.get(n).map_err(store_error)
}
fn unsigned(value: i64) -> Result<u64, ApiError> {
    value.try_into().map_err(|_| corrupt())
}
fn signed(value: u64) -> Result<i64, ApiError> {
    value
        .try_into()
        .map_err(|_| invalid("wake intent integer overflow"))
}
fn phase(text: &str) -> Result<WakeIntentPhase, ApiError> {
    Ok(match text {
        "prepared" => WakeIntentPhase::Prepared,
        "armed" => WakeIntentPhase::Armed,
        "accepted" => WakeIntentPhase::Accepted,
        "unknown" => WakeIntentPhase::Unknown,
        "no_effect" => WakeIntentPhase::NoEffect,
        _ => return Err(corrupt()),
    })
}
fn publication(r: &Row<'_>, seq: usize) -> Result<Option<LogicalPublicationKey>, ApiError> {
    match (get::<Option<i64>>(r, seq)?, get::<Option<i64>>(r, seq + 1)?) {
        (None, None) => Ok(None),
        (Some(a), Some(b)) => Ok(Some(LogicalPublicationKey {
            decision_seq: unsigned(a)?,
            event_offset: unsigned(b)?,
        })),
        _ => Err(corrupt()),
    }
}
fn decode(r: &Row<'_>) -> Result<WakeIntentSnapshot, ApiError> {
    let scope: WakeScope =
        serde_json::from_slice(&get::<Vec<u8>>(r, 10)?).map_err(|_| corrupt())?;
    let adapter: String = get(r, 2)?;
    let owner: String = get(r, 22)?;
    let nonce: Option<String> = get(r, 23)?;
    Ok(WakeIntentSnapshot {
        ordinal: unsigned(get(r, 0)?)?,
        attempt: WakeAttemptId::parse(get::<String>(r, 1)?).map_err(|_| corrupt())?,
        origin_seat: SeatId::parse(get::<String>(r, 4)?).map_err(|_| corrupt())?,
        origin_host: HostId::from_row(get(r, 5)?),
        origin_host_boot: HostBootId::parse(get::<String>(r, 6)?).map_err(|_| corrupt())?,
        origin_host_epoch: unsigned(get(r, 7)?)?,
        origin_target: HostTargetId::parse(get::<String>(r, 8)?).map_err(|_| corrupt())?,
        origin_target_generation: unsigned(get(r, 9)?)?,
        preparation: WakeIntentPreparation {
            adapter: builtins().agent(&adapter).map_err(|_| corrupt())?,
            replay_scope: WakeReplayScope::parse(get(r, 3)?).map_err(|_| corrupt())?,
            target: WakeControlTarget {
                scope,
                descriptor: get(r, 11)?,
            },
            correlation: get(r, 12)?,
            side_sessions: get::<Option<String>>(r, 36)?
                .map(|text| {
                    serde_json::from_str::<Vec<String>>(&text)
                        .map(|ids| {
                            ids.into_iter()
                                .map(crate::protocol::ids::NativeSessionId::new)
                                .collect()
                        })
                        .map_err(|_| corrupt())
                })
                .transpose()?
                .unwrap_or_default(),
        },
        frontier: LogicalAttentionFrontier {
            invitation: publication(r, 13)?,
            addressed_receipt: publication(r, 15)?,
            actionable_warning: publication(r, 17)?,
            lazy: get(r, 19)?,
        },
        phase: phase(&get::<String>(r, 20)?)?,
        revision: unsigned(get(r, 21)?)?,
        owner_boot: uuid::Uuid::parse_str(&owner).map_err(|_| corrupt())?,
        arm_nonce: nonce
            .map(|v| uuid::Uuid::parse_str(&v).map_err(|_| corrupt()))
            .transpose()?,
        cleanup_quarantined: get(r, 24)?,
        diagnostic: get(r, 25)?,
        ids_complete: get(r, 26)?,
        logical: match get::<String>(r, 27)?.as_str() {
            "active" => crate::ports::WakeIntentLogical::Active,
            "scope_ended" => crate::ports::WakeIntentLogical::ScopeEnded,
            "fulfilled" => crate::ports::WakeIntentLogical::Fulfilled {
                turn_id: crate::protocol::ids::NativeTurnId::parse(get::<String>(r, 28)?)
                    .map_err(|_| corrupt())?,
                ordinal: unsigned(get(r, 29)?)?,
            },
            _ => return Err(corrupt()),
        },
        arm_baseline: match (get::<Option<String>>(r, 30)?, get::<Option<i64>>(r, 31)?) {
            (None, None) => None,
            (Some(id), Some(ordinal)) => Some(crate::ports::WakeNativeBaseline {
                id: crate::protocol::ids::NativeTurnId::parse(id).map_err(|_| corrupt())?,
                ordinal: unsigned(ordinal)?,
            }),
            _ => return Err(corrupt()),
        },
        helper: get::<Option<Vec<u8>>>(r, 32)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| corrupt()))
            .transpose()?,
        cleanup_list_attempted: get(r, 33)?,
        cleanup_pending_ids: Vec::new(),
        native_no_add_exception: get(r, 34)?,
        add_helper: get::<Option<Vec<u8>>>(r, 35)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| corrupt()))
            .transpose()?,
        owned_ids: vec![],
    })
}
fn load(
    db: &Connection,
    instance: &str,
    attempt: &WakeAttemptId,
) -> Result<Option<WakeIntentSnapshot>, ApiError> {
    let mut stmt = db
        .prepare(&format!(
            "SELECT {COLUMNS} FROM wake_intents WHERE instance_id=?1 AND attempt=?2"
        ))
        .map_err(store_error)?;
    let mut rows = stmt
        .query(params![instance, attempt.as_str()])
        .map_err(store_error)?;
    let Some(row) = rows.next().map_err(store_error)? else {
        return Ok(None);
    };
    let mut result = decode(row)?;
    let mut ids=db.prepare("SELECT queue_id FROM wake_intent_ids WHERE intent_ordinal=?1 ORDER BY queue_id LIMIT 129").map_err(store_error)?;
    let mut pending=db.prepare("SELECT queue_id FROM wake_intent_ids WHERE intent_ordinal=?1 AND cleanup_attempted=0 ORDER BY queue_id LIMIT 129").map_err(store_error)?;
    result.cleanup_pending_ids = pending
        .query_map([signed(result.ordinal)?], |r| r.get(0))
        .map_err(store_error)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(store_error)?;
    result.owned_ids = ids
        .query_map([signed(result.ordinal)?], |r| r.get(0))
        .map_err(store_error)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(store_error)?;
    if result.owned_ids.len() > MAX_IDS {
        return Err(corrupt());
    }
    Ok(Some(result))
}
fn matching(row: &WakeIntentSnapshot, r: &WakeReservation, p: &WakeIntentPreparation) -> bool {
    row.attempt == r.attempt
        && row.origin_seat == r.seat
        && row.origin_host_boot == r.host_boot
        && row.origin_host_epoch == r.host_epoch
        && row.origin_target == r.target
        && row.origin_target_generation == r.target_generation
        && row.frontier == r.attention_witness.frontier()
        && row.owner_boot == r.daemon_boot
        && row.preparation == *p
}
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b.is_ascii_graphic())
}
fn validate_preparation(p: &WakeIntentPreparation) -> Result<(), ApiError> {
    if p.target.descriptor.len() > 16384
        || !valid_id(&p.correlation)
        || p.target.scope.kernel_boot.is_empty()
        || p.target.scope.kernel_boot.len() > 64
        || p.target.scope.process.pid == 0
    {
        return Err(invalid("invalid wake intent preparation"));
    }
    let control = builtins()
        .by_id(p.adapter)
        .map_err(|_| invalid("unregistered wake adapter"))?
        .wake_control()
        .ok_or_else(|| invalid("wake control unavailable"))?;
    if control.replay_scope(&p.target)? != p.replay_scope {
        return Err(invalid("wake replay scope mismatch"));
    }
    Ok(())
}
fn canonical(
    store: &SqliteStore,
    db: &Connection,
    r: &WakeReservation,
    p: &WakeIntentPreparation,
) -> Result<bool, ApiError> {
    if store.settings.daemon_boot != Some(r.daemon_boot)
        || store.context.clock().monotonic_now() > r.lease_until
        || !wake::validate_reservation(db, &store.instance, r)?
    {
        return Ok(false);
    }
    let s = &p.target.scope;
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id JOIN node_kernel_boot k ON k.id=1 WHERE b.binding_id=?1 AND s.instance_id=?2 AND b.seat_id=?3 AND b.harness=?4 AND b.native_session=?5 AND b.execution_id=?6 AND b.generation=?7 AND s.generation=b.generation AND b.role='top_level' AND b.ended_at IS NULL AND b.liveness='connected' AND b.registered_at IS NOT NULL AND b.observation_provenance='cooperative_top_level' AND b.liveness_pid=?8 AND b.liveness_start=?9 AND b.liveness_boot=?10 AND k.boot=b.liveness_boot AND b.target_id=s.target_id AND b.target_generation=s.target_generation)",params![s.binding.as_str(),store.instance,r.seat.as_str(),p.adapter.as_str(),s.native_session.as_str(),s.execution.as_str(),signed(s.generation)?,i64::from(s.process.pid),signed(s.process.start)?,s.kernel_boot],|r|r.get(0)).map_err(store_error)?;
    Ok(exact
        && r.authority
            .targets()
            .iter()
            .any(|t| t.binding == s.binding && !t.dormant)
        && wake::wake_owed_to_binding(db, r, Some(&s.binding))?)
}
fn frontier_key(
    key: Option<LogicalPublicationKey>,
) -> Result<(Option<i64>, Option<i64>), ApiError> {
    key.map(|k| Ok((Some(signed(k.decision_seq)?), Some(signed(k.event_offset)?))))
        .unwrap_or(Ok((None, None)))
}
pub(super) fn prepare(
    store: &SqliteStore,
    r: &WakeReservation,
    p: &WakeIntentPreparation,
    budget: &CallBudget,
) -> Result<PrepareWakeIntent, ApiError> {
    validate_preparation(p)?;
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(&mut writer,budget,|tx| {
        if let Some(row)=load(tx,&store.instance,&r.attempt)? {
            return if matching(&row,r,p) {Ok(Some(PrepareWakeIntent::Existing(row)))} else {Err(invalid("wake intent frozen fields differ"))};
        }
        if !canonical(store,tx,r,p)? {return Ok(Some(PrepareWakeIntent::Refused))}
        Ok(None)
    },|tx,at,known| {
        if let Some(known)=known {return Ok(known)}
        // Clean no-effect or finitely cleaned retired rows can leave history. Never
        // prune Armed or a still-referenced attempt a reservation could address.
        tx.execute("DELETE FROM wake_intents WHERE ordinal IN (SELECT i.ordinal FROM wake_intents i WHERE i.instance_id=?1 AND (i.phase='no_effect' OR (i.logical_status!='active' AND i.cleanup_list_attempted=1 AND NOT EXISTS(SELECT 1 FROM wake_intent_ids q WHERE q.intent_ordinal=i.ordinal AND q.cleanup_attempted=0))) AND i.phase!='armed' AND i.cleanup_quarantined=0 AND NOT EXISTS(SELECT 1 FROM wake_work w JOIN seats s ON s.id=w.seat_id WHERE s.instance_id=i.instance_id AND (w.reservation_id=i.attempt OR w.last_reservation_id=i.attempt)) ORDER BY i.ordinal LIMIT 32)",[&store.instance]).map_err(store_error)?;
        let count:i64=tx.query_row("SELECT count(*) FROM wake_intents WHERE instance_id=?1",[&store.instance],|r|r.get(0)).map_err(store_error)?;
        let blocked:bool=tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM wake_intents WHERE instance_id=?1 AND adapter=?2 AND replay_scope=?3 AND {UNRESOLVED})"),params![store.instance,p.adapter.as_str(),p.replay_scope.as_str()],|r|r.get(0)).map_err(store_error)?;
        if count>=MAX_INTENTS || blocked {return Ok(PrepareWakeIntent::Refused)}
        let side_json:Option<String>=if p.side_sessions.is_empty(){None}else{Some(serde_json::to_string(&p.side_sessions.iter().map(|s|s.as_str()).collect::<Vec<_>>()).map_err(|_|invalid("invalid side sessions"))?)};
        let scope=serde_json::to_vec(&p.target.scope).map_err(|_|invalid("invalid wake scope"))?;
        let f=r.attention_witness.frontier();
        let (a,b)=frontier_key(f.invitation)?;let(c,d)=frontier_key(f.addressed_receipt)?;let(e,f)=frontier_key(f.actionable_warning)?;
        let host:i64=tx.query_row("SELECT host_id FROM seats WHERE id=?1",[r.seat.as_str()],|r|r.get(0)).map_err(store_error)?;
        tx.execute("INSERT INTO wake_intents(instance_id,attempt,adapter,replay_scope,origin_seat,origin_host,origin_host_boot,origin_host_epoch,origin_target,origin_target_generation,target_json,descriptor,correlation,invitation_seq,invitation_offset,receipt_seq,receipt_offset,warning_seq,warning_offset,lazy_ordinal,phase,revision,owner_boot,created_at,updated_at,cleanup_quarantined,diagnostic,side_sessions) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,'prepared',1,?21,?22,?22,0,'',?23)",params![store.instance,r.attempt.as_str(),p.adapter.as_str(),p.replay_scope.as_str(),r.seat.as_str(),host,r.host_boot.as_str(),signed(r.host_epoch)?,r.target.as_str(),signed(r.target_generation)?,scope,p.target.descriptor,p.correlation,a,b,c,d,e,f,r.attention_witness.frontier().lazy,r.daemon_boot.to_string(),at.utc.0,side_json]).map_err(store_error)?;
        Ok(PrepareWakeIntent::Prepared(load(tx,&store.instance,&r.attempt)?.ok_or_else(corrupt)?))
    })
}
pub(super) fn arm(
    store: &SqliteStore,
    r: &WakeReservation,
    revision: u64,
    nonce: uuid::Uuid,
    budget: &CallBudget,
) -> Result<Option<WakeArmTicket>, ApiError> {
    if nonce.is_nil() {
        return Err(invalid("invalid wake arm nonce"));
    }
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(&mut writer,budget,|tx| {
        let Some(row)=load(tx,&store.instance,&r.attempt)? else {return Ok(None)};
        if row.phase!=WakeIntentPhase::Prepared || !row.cleanup_quarantined || row.logical!=crate::ports::WakeIntentLogical::Active || row.revision!=revision || !matching(&row,r,&row.preparation) || !canonical(store,tx,r,&row.preparation)? {return Ok(None)}
        // The seat's opt-in is re-read at arm: flipped off since selection is the stale-authority path.
        if !super::seat_config::programmatic_wake_on(tx,&r.seat) {return Ok(None)}
        let Some(helper)=row.helper.as_ref().filter(|h|h.owner_boot==r.daemon_boot && h.purpose==crate::harness::wake::WakeHelperPurpose::Add && h.witness.is_some()) else {return Ok(None)};
        if helper.kernel_boot!=row.preparation.target.scope.kernel_boot {return Ok(None)}
        let Some(native)=super::native_turn::load(tx,&store.instance,&row.preparation.target.scope.binding)? else {return Ok(None)};
        if native.scope!=row.preparation.target.scope || native.phase!=crate::ports::NativeTurnPhase::Ended {return Ok(None)}
        Ok(Some(native))
    },|tx,at,native| {
        let Some(native)=native else{return Ok(None)};
        let next=revision.checked_add(1).ok_or_else(corrupt)?;
        let changed=tx.execute("UPDATE wake_intents SET phase='armed',revision=?3,arm_nonce=?4,updated_at=?5,arm_native_id=?7,arm_native_ordinal=?8,add_helper_json=helper_json,armed_at_utc=?5 WHERE instance_id=?1 AND attempt=?2 AND phase='prepared' AND revision=?6",params![store.instance,r.attempt.as_str(),signed(next)?,nonce.to_string(),at.utc.0,signed(revision)?,native.id.as_str(),signed(native.ordinal)?]).map_err(store_error)?;
        Ok((changed==1).then(||WakeArmTicket {attempt:r.attempt.clone(),revision:next,owner_boot:r.daemon_boot,nonce}))
    })
}
pub(super) fn record_result(
    store: &SqliteStore,
    ticket: &WakeArmTicket,
    result: &WakeIntentResult,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    if result.diagnostic.len() > 2048 {
        return Err(invalid("wake intent diagnostic too large"));
    }
    let (phase, id) = match &result.outcome {
        WakeIntentOutcome::Accepted { id } => {
            if !valid_id(id) {
                return Err(invalid("invalid owned queue id"));
            }
            ("accepted", Some(id))
        }
        WakeIntentOutcome::Unknown => ("unknown", None),
        WakeIntentOutcome::NoEffect => ("no_effect", None),
    };
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(&mut writer,budget,|tx| {
        let row=load(tx,&store.instance,&ticket.attempt)?;
        Ok(row.filter(|r|r.phase==WakeIntentPhase::Armed && r.owner_boot==ticket.owner_boot && r.arm_nonce==Some(ticket.nonce) && store.settings.daemon_boot==Some(ticket.owner_boot)))
    },|tx,at,row| {
        let Some(row)=row else {return Ok(false)};
        if let Some(id)=id { insert_ids(tx,&row,std::slice::from_ref(id))?; }
        // Armed forbids helper replacement; a stale negative cannot undo positive
        // quiescence already committed for this submission helper.
        tx.execute("UPDATE wake_intents SET phase=?3,revision=revision+1,cleanup_quarantined=cleanup_quarantined AND ?4,diagnostic=?5,updated_at=?6 WHERE instance_id=?1 AND attempt=?2",params![store.instance,ticket.attempt.as_str(),phase,!result.cleanup_confirmed,result.diagnostic,at.utc.0]).map_err(store_error)?;
        Ok(true)
    })
}
fn insert_ids(db: &Connection, row: &WakeIntentSnapshot, ids: &[String]) -> Result<(), ApiError> {
    let mut all: BTreeSet<&str> = row.owned_ids.iter().map(String::as_str).collect();
    for id in ids {
        if !valid_id(id) {
            return Err(invalid("invalid owned queue id"));
        }
        all.insert(id);
    }
    if all.len() > MAX_IDS {
        return Err(invalid("owned queue id bound exceeded"));
    }
    for id in ids {
        db.execute(
            "INSERT OR IGNORE INTO wake_intent_ids(intent_ordinal,queue_id) VALUES(?1,?2)",
            params![signed(row.ordinal)?, id],
        )
        .map_err(store_error)?;
    }
    Ok(())
}
pub(super) fn record_ids(
    store: &SqliteStore,
    attempt: &WakeAttemptId,
    revision: u64,
    ids: &[String],
    complete: bool,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    if ids.len() > MAX_IDS {
        return Err(invalid("owned queue id bound exceeded"));
    }
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(&mut writer,budget,|tx|Ok(load(tx,&store.instance,attempt)?.filter(|r|r.revision==revision && (matches!(r.phase,WakeIntentPhase::Armed|WakeIntentPhase::Accepted|WakeIntentPhase::Unknown) || (r.logical!=crate::ports::WakeIntentLogical::Active && r.phase!=WakeIntentPhase::Armed && r.helper.as_ref().is_some_and(|h|h.purpose==crate::harness::wake::WakeHelperPurpose::List))))),|tx,at,row| {
        let Some(row)=row else {return Ok(false)};
        insert_ids(tx,&row,ids)?;
        tx.execute("UPDATE wake_intents SET revision=revision+1,ids_complete=?3,updated_at=?4 WHERE instance_id=?1 AND attempt=?2",params![store.instance,attempt.as_str(),complete,at.utc.0]).map_err(store_error)?;
        Ok(true)
    })
}
pub(super) fn refuse_prepared(
    store: &SqliteStore,
    attempt: &WakeAttemptId,
    revision: u64,
    cleanup_confirmed: bool,
    diagnostic: &str,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    if diagnostic.len() > 2048 {
        return Err(invalid("wake intent diagnostic too large"));
    }
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(
        &mut writer,
        budget,
        |tx| {
            Ok(load(tx, &store.instance, attempt)?.filter(|row| {
                row.phase == WakeIntentPhase::Prepared
                    && row.revision == revision
                    && store.settings.daemon_boot == Some(row.owner_boot)
            }))
        },
        |tx, at, row| {
            let Some(row) = row else {
                return Ok(false);
            };
            // Exact-helper positive evidence survives delayed refusal metadata.
            // Helperless legacy uncertainty has no such per-operation evidence.
            let quarantine = !cleanup_confirmed
                && (row.helper.is_none() || row.cleanup_quarantined);
            // This CAS revokes unarmed authority. An uninitialized Add helper is
            // resource uncertainty only; never erase quarantine or a committed witness.
            let native_exception = quarantine && row.arm_nonce.is_none()
                && row.helper.as_ref().is_some_and(|helper| {
                    helper.purpose == crate::harness::wake::WakeHelperPurpose::Add
                        && helper.witness.is_none()
                        && helper.owner_boot == row.owner_boot
                });
            tx.execute(
                "UPDATE wake_intents SET phase='no_effect',revision=revision+1,cleanup_quarantined=?3,diagnostic=?4,updated_at=?5,native_no_add_exception=?6 WHERE instance_id=?1 AND attempt=?2",
                params![store.instance, attempt.as_str(), quarantine, diagnostic, at.utc.0, native_exception],
            ).map_err(store_error)?;
            Ok(true)
        },
    )
}
pub(super) fn recover(
    store: &SqliteStore,
    attempt: &WakeAttemptId,
    revision: u64,
    prior_boot: uuid::Uuid,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(&mut writer,budget,|tx| {
        if !store.settings.daemon_boot.is_some_and(|b|b!=prior_boot){return Ok(None)}
        Ok(load(tx,&store.instance,attempt)?.filter(|r|r.revision==revision && (
            (r.owner_boot==prior_boot && matches!(r.phase,WakeIntentPhase::Prepared|WakeIntentPhase::Armed)) ||
            (r.logical!=crate::ports::WakeIntentLogical::Active && r.cleanup_quarantined && !r.native_no_add_exception && r.helper.as_ref().is_some_and(|h|h.owner_boot==prior_boot && h.witness.is_none() && matches!(h.purpose,crate::harness::wake::WakeHelperPurpose::List|crate::harness::wake::WakeHelperPurpose::Delete)))
        )))
    },|tx,at,row| {
        let Some(row)=row else{return Ok(false)};
        if matches!(row.phase,WakeIntentPhase::Prepared|WakeIntentPhase::Armed) && row.owner_boot==prior_boot {
            let unarmed=row.phase==WakeIntentPhase::Prepared && row.arm_nonce.is_none();
            // Recovery changes submission classification, not positive cleanup evidence
            // for the retained exact helper. Helperless legacy Armed rows stay conservative.
            let uncertain=row.cleanup_quarantined || (!unarmed && row.helper.is_none());
            tx.execute("UPDATE wake_intents SET phase=?3,revision=revision+1,cleanup_quarantined=?6,native_no_add_exception=?4,diagnostic='prior helper recovery',updated_at=?5 WHERE instance_id=?1 AND attempt=?2",params![store.instance,attempt.as_str(),if unarmed {"no_effect"} else {"unknown"},unarmed && uncertain,at.utc.0,uncertain]).map_err(store_error)?;
        } else {
            // The cleanup invocation never initialized. Its resource uncertainty is not
            // a new enqueue; keep original effect history and retirement unchanged.
            tx.execute("UPDATE wake_intents SET native_no_add_exception=1,revision=revision+1,diagnostic='uninitialized prior cleanup helper',updated_at=?3 WHERE instance_id=?1 AND attempt=?2",params![store.instance,attempt.as_str(),at.utc.0]).map_err(store_error)?;
        }
        Ok(true)
    })
}
pub(super) fn snapshot(
    store: &SqliteStore,
    attempt: &WakeAttemptId,
    budget: &CallBudget,
) -> Result<Option<WakeIntentSnapshot>, ApiError> {
    let db = store.context.open_query(budget.clone())?;
    db.execute_batch("BEGIN DEFERRED").map_err(store_error)?;
    load(&db, &store.instance, attempt)
}
pub(super) fn page(
    store: &SqliteStore,
    seat: Option<&SeatId>,
    after: u64,
    limit: u16,
    budget: &CallBudget,
) -> Result<WakeIntentPage, ApiError> {
    if limit == 0 || limit > 32 {
        return Err(invalid("invalid wake intent page limit"));
    }
    let db = store.context.open_query(budget.clone())?;
    let filter = if seat.is_some() {
        " AND origin_seat=?4"
    } else {
        ""
    };
    let selection = if seat.is_some() { UNRESOLVED } else { "1" };
    let sql = format!(
        "SELECT ordinal,attempt,origin_seat,phase,revision,cleanup_quarantined FROM wake_intents WHERE instance_id=?1 AND ordinal>?2 AND {selection}{filter} ORDER BY ordinal LIMIT ?3"
    );
    let mut stmt = db.prepare(&sql).map_err(store_error)?;
    let mut rows = if let Some(seat) = seat {
        stmt.query(params![
            store.instance,
            signed(after)?,
            i64::from(limit) + 1,
            seat.as_str()
        ])
    } else {
        stmt.query(params![
            store.instance,
            signed(after)?,
            i64::from(limit) + 1
        ])
    }
    .map_err(store_error)?;
    let mut items = vec![];
    while let Some(r) = rows.next().map_err(store_error)? {
        items.push(WakeIntentSummary {
            ordinal: unsigned(get(r, 0)?)?,
            attempt: WakeAttemptId::parse(get::<String>(r, 1)?).map_err(|_| corrupt())?,
            origin_seat: SeatId::parse(get::<String>(r, 2)?).map_err(|_| corrupt())?,
            phase: phase(&get::<String>(r, 3)?)?,
            revision: unsigned(get(r, 4)?)?,
            cleanup_quarantined: get(r, 5)?,
        });
    }
    let next_after = if items.len() > usize::from(limit) {
        items.pop();
        items.last().map(|r| r.ordinal)
    } else {
        None
    };
    Ok(WakeIntentPage { items, next_after })
}

mod activation;
pub(super) use activation::*;

mod selection;
pub(super) use selection::queue_wake_selection;

pub(crate) mod native_fallback;
pub(crate) use native_fallback::{
    claim as claim_native_fallback, degraded_at_for_seat, finish as finish_native_fallback,
};
