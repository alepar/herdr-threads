//! Durable wake attempts. The scheduler owns process-monotonic eligibility.

use super::{
    connection::{StoreContext, api_error, store_error},
    effective::{self, EffectiveSeatAttention},
    poke,
};
use crate::{
    ports::{
        LogicalAttentionFrontier, LogicalPublicationKey, PokeDue, PokeReceipt, PokeReservation,
        PriorLadder, RefusalCause, ReservedWakeAuthority, WakeAttentionWitness, WakeCandidate,
        WakeOutcome, WakeRecoveryOutcome, WakeRecoveryRequest, WakeReservation,
        WarningOfferFrontier,
    },
    protocol::{
        ids::{ExecutionId, HostBootId, HostTargetId, SeatId, TerminalId, WakeAttemptId},
        results::{ApiError, ErrorCode},
        summary::SummarySettings,
        time::{CallBudget, MonoInstant, UtcMillis},
    },
};
use rusqlite::{Connection, OptionalExtension, params};

/// The write every new-attention producer makes: set the attention reason bit
/// and advance `attention_version` so the dispatcher re-derives the seat's
/// wake reasons. Catch-up release (ht-1ip.6) calls it in the row-end
/// transaction.
pub fn note_new_attention(tx: &rusqlite::Transaction<'_>, seat: &SeatId) -> Result<(), ApiError> {
    tx.execute(
        "INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES (?1,1,1) ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|1,attention_version=attention_version+1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    Ok(())
}

fn nonnegative(value: i64) -> Result<u64, ApiError> {
    u64::try_from(value).map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative wake scalar"))
}

/// Build one candidate from a complete canonical attention scan in the same
/// read transaction. A partial attention slice must never call this function.
pub fn load_candidate(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    attention: &EffectiveSeatAttention,
    scan_decision_seq: i64,
) -> Result<WakeCandidate, ApiError> {
    let (state,target,generation,target_generation,boot,epoch,decision_seq,episode,open):(String,Option<String>,i64,i64,Option<String>,i64,i64,i64,i64)=db.query_row(
        "SELECT s.state,s.target_id,s.generation,s.target_generation,h.host_boot,h.host_epoch,h.decision_seq,s.unavailability_episode,s.unavailability_open FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1 AND s.instance_id=?2",
        params![seat.as_str(),instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).map_err(store_error)?;
    if decision_seq != scan_decision_seq {
        return Err(api_error(
            ErrorCode::CursorStale,
            "seat attention decision changed",
        ));
    }
    let observation = if let Some(target) = target.as_deref() {
        effective::effective_observation(db, instance, target)?
    } else {
        None
    };
    let held: bool = if let Some(target) = target.as_deref() {
        db.query_row(
        "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE instance_id=?1 AND target_id=?2 AND released_at IS NULL)",
        params![instance,target],|r|r.get(0)).map_err(store_error)?
    } else {
        false
    };
    let continuity_resolved = state == "resolved"
        && !held
        && observation.as_ref().is_some_and(|o| {
            boot.as_deref() == Some(o.host_boot.as_str())
                && epoch == o.epoch
                && target_generation == o.structural_generation
        });
    type BindingColumns = Option<(i64, i64, String, String, String, i64, Option<i64>)>;
    let binding:BindingColumns=db.query_row(
        "SELECT generation,target_generation,target_id,execution_id,host_boot,host_epoch,registered_at FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
        [seat.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(store_error)?;
    let current_binding = binding.filter(
        |(bound_generation, bound_target, bound_target_id, _, bound_boot, bound_epoch, _)| {
            *bound_generation == generation
                && *bound_target == target_generation
                && target.as_deref() == Some(bound_target_id.as_str())
                && boot.as_deref() == Some(bound_boot.as_str())
                && *bound_epoch == epoch
        },
    );
    let binding_generation = current_binding
        .as_ref()
        .map(|(generation, _, _, _, _, _, _)| nonnegative(*generation))
        .transpose()?;
    let binding_execution = current_binding
        .as_ref()
        .map(|(_, _, _, execution, _, _, _)| ExecutionId::new(execution));
    type WakeRow = (
        i64,
        i64,
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        i64,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );
    let wake:Option<WakeRow>=db.query_row("SELECT reason_bits,attention_version,checkpoint_version,retry_step,reservation_id,reservation_boot,last_reservation_id,last_reservation_boot,minimum_delay_ms,effective_delay_ms,last_outcome,last_reserved_at_utc,last_invitation_seq,last_invitation_offset,last_receipt_seq,last_receipt_offset,last_warning_seq,last_warning_offset FROM wake_work WHERE seat_id=?1",
        [seat.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?,r.get(13)?,r.get(14)?,r.get(15)?,r.get(16)?,r.get(17)?))).optional().map_err(store_error)?;
    let (
        reason_bits,
        attention_version,
        checkpoint_version,
        retry_step,
        reservation_id,
        reservation_boot,
        last_reservation_id,
        last_reservation_boot,
        minimum_delay_ms,
        effective_delay_ms,
        last_outcome,
        last_reserved_at_utc,
        last_invitation_seq,
        last_invitation_offset,
        last_receipt_seq,
        last_receipt_offset,
        last_warning_seq,
        last_warning_offset,
    ) = wake.unwrap_or((
        0, 0, 0, 0, None, None, None, None, 0, 0, None, None, None, None, None, None, None, None,
    ));
    let last_key = |seq: Option<i64>,
                    offset: Option<i64>|
     -> Result<Option<LogicalPublicationKey>, ApiError> {
        match (seq, offset) {
            (None, None) => Ok(None),
            (Some(seq), Some(offset)) if seq > 0 && offset >= 0 => {
                Ok(Some(LogicalPublicationKey {
                    decision_seq: nonnegative(seq)?,
                    event_offset: nonnegative(offset)?,
                }))
            }
            _ => Err(api_error(
                ErrorCode::StoreCorrupt,
                "invalid retained wake frontier",
            )),
        }
    };
    let last_reserved_frontier = LogicalAttentionFrontier {
        invitation: last_key(last_invitation_seq, last_invitation_offset)?,
        addressed_receipt: last_key(last_receipt_seq, last_receipt_offset)?,
        actionable_warning: last_key(last_warning_seq, last_warning_offset)?,
    };
    let offer:Option<(i64,String,i64)>=db.query_row("SELECT binding_generation,execution_id,offered_through_seq FROM warning_offer WHERE seat_id=?1",
        [seat.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?;
    let warning_offer = offer
        .map(|(generation, execution, seq)| {
            Ok(WarningOfferFrontier {
                generation: nonnegative(generation)?,
                execution: ExecutionId::new(execution),
                offered_through_seq: nonnegative(seq)?,
            })
        })
        .transpose()?;
    let latest_warning_seq = attention.latest_warning_seq.map(nonnegative).transpose()?;
    let witness = WakeAttentionWitness::from_complete(
        instance.into(),
        seat.clone(),
        nonnegative(decision_seq)?,
        attention.has_pending_invitation,
        attention.has_pending_receipt,
        latest_warning_seq,
        nonnegative(episode)?,
        open != 0,
        attention.frontier,
    );
    Ok(WakeCandidate {
        seat: seat.clone(),
        attention_witness: Some(witness),
        effectively_retired: state == "retired",
        continuity_resolved,
        binding_generation,
        binding_execution,
        target: target.map(HostTargetId::new),
        reason_bits: nonnegative(reason_bits)?,
        has_pending_invitation: attention.has_pending_invitation,
        has_pending_receipt: attention.has_pending_receipt,
        actionable_warning_generation: if attention.latest_warning_seq.is_some() {
            binding_generation
        } else {
            None
        },
        actionable_warning_seq: latest_warning_seq,
        warning_offer,
        attention_version: nonnegative(attention_version)?,
        checkpoint_version: nonnegative(checkpoint_version)?,
        retry_step: u32::try_from(retry_step)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid retry step"))?,
        reservation_id: reservation_id.map(WakeAttemptId::new),
        reservation_boot: reservation_boot.map(HostBootId::new),
        last_reservation_id: last_reservation_id.map(WakeAttemptId::new),
        last_reservation_boot: last_reservation_boot.map(HostBootId::new),
        last_reserved_frontier,
        minimum_delay_ms: nonnegative(minimum_delay_ms)?,
        effective_delay_ms: nonnegative(effective_delay_ms)?,
        last_outcome,
        last_reserved_at_utc: last_reserved_at_utc.map(UtcMillis),
    })
}

/// SQL true when an open binding on `seat_expr` belongs to a person (`me init`):
/// such a seat is never prompted (TRUST-POLICY A4).
pub(super) fn human_bound_sql(seat_expr: &str) -> String {
    format!(
        "EXISTS(SELECT 1 FROM occupant_bindings hb WHERE hb.seat_id={seat_expr} AND hb.ended_at IS NULL AND hb.harness='human')"
    )
}

/// SQL true when `seat_expr` can pass a wake or poke reservation without its
/// own state changing first: resolved, with a target that has no unreleased
/// recovery hold, and not bound to a person. The observation-dependent half
/// of the authority (boot, epoch, generation, UI) is rechecked at
/// reservation time only.
pub(super) fn reservable_seat_sql(seat_expr: &str) -> String {
    format!(
        "(EXISTS(SELECT 1 FROM seats rs WHERE rs.id={seat_expr} AND rs.state='resolved' AND rs.target_id IS NOT NULL \
         AND NOT EXISTS(SELECT 1 FROM recovery_holds rh WHERE rh.instance_id=rs.instance_id AND rh.target_id=rs.target_id AND rh.released_at IS NULL)) \
         AND NOT {})",
        human_bound_sql(seat_expr)
    )
}

// Allowed: authority lookup keyed by every component of the wake reservation.
#[allow(clippy::too_many_arguments)]
fn current_authority(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    target: &HostTargetId,
    generation: u64,
    target_generation: u64,
    boot: &HostBootId,
    epoch: u64,
) -> Result<Option<ReservedWakeAuthority>, ApiError> {
    // A person's pane identity (`me init`) is never prompted: its mail waits
    // for the person to read and ACK it by hand.
    let human: bool = db
        .query_row(
            &format!("SELECT {}", human_bound_sql("?1")),
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if human {
        return Ok(None);
    }
    let Some(observation) = effective::effective_observation(db, instance, target.as_str())? else {
        return Ok(None);
    };
    if observation.host_boot != boot.as_str()
        || nonnegative(observation.epoch)? != epoch
        || nonnegative(observation.structural_generation)? != target_generation
    {
        return Ok(None);
    }
    let Some(execution) = observation
        .verified_execution
        .as_ref()
        .filter(|id| !id.is_empty())
    else {
        return cooperative_authority(
            db,
            seat,
            target,
            generation,
            target_generation,
            &observation,
        );
    };
    if observation.occupancy != "occupied"
        || observation.ui_state != "idle"
        || !observation.top_level_occupant
    {
        return Ok(None);
    }
    type BindingColumns = Option<(i64, i64, String, String, String, i64, Option<i64>)>;
    let binding:BindingColumns=db.query_row(
        "SELECT generation,target_generation,target_id,execution_id,host_boot,host_epoch,registered_at FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
        [seat.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(store_error)?;
    if let Some((
        bound_generation,
        bound_target,
        bound_target_id,
        bound_execution,
        bound_boot,
        bound_epoch,
        registered_at,
    )) = binding
    {
        if nonnegative(bound_generation)? != generation
            || nonnegative(bound_target)? != target_generation
            || bound_target_id != target.as_str()
            || bound_execution != *execution
            || bound_boot != boot.as_str()
            || nonnegative(bound_epoch)? != epoch
        {
            return Ok(None);
        };
        if registered_at.is_some() {
            return Ok(Some(ReservedWakeAuthority::Registered {
                binding_generation: generation,
                execution: ExecutionId::new(execution),
            }));
        }
    }
    Ok(Some(ReservedWakeAuthority::RecoveryHint {
        execution: ExecutionId::new(execution),
    }))
}

/// Cooperative native policy (same reasoning as the D2 registered move):
/// Unknown current execution never blocks structural identity. The effective
/// observation names a terminal in a verified server incarnation with no
/// positive evidence of an empty shell, an active turn, blocked UI or human
/// input; the seat's open binding is required (none: no authority) and is the
/// seat's current generation on this target and names that same terminal and
/// incarnation. Whether the
/// occupant is a recognized idle harness (never a shell or unknown harness)
/// is the host adapter's recheck immediately before prompting.
fn cooperative_authority(
    db: &Connection,
    seat: &SeatId,
    target: &HostTargetId,
    generation: u64,
    target_generation: u64,
    observation: &effective::EffectiveObservation,
) -> Result<Option<ReservedWakeAuthority>, ApiError> {
    if observation.occupancy == "empty_shell"
        || matches!(
            observation.ui_state.as_str(),
            "active_turn" | "approval_or_question" | "human_input"
        )
        || observation.incarnation_source_kind.is_none()
    {
        return Ok(None);
    }
    let (Some(terminal), Some(incarnation)) = (
        observation.terminal_id.as_ref().filter(|id| !id.is_empty()),
        observation.incarnation.as_ref().filter(|id| !id.is_empty()),
    ) else {
        return Ok(None);
    };
    type BindingColumns = Option<(
        i64,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<i64>,
        String,
    )>;
    let binding: BindingColumns = db.query_row(
        "SELECT generation,target_generation,target_id,terminal_id,incarnation,registered_at,harness FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
        [seat.as_str()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
    ).optional().map_err(store_error)?;
    let (binding_generation, harness) = match binding {
        Some((
            bound_generation,
            bound_target,
            bound_target_id,
            bound_terminal,
            bound_incarnation,
            registered_at,
            bound_harness,
        )) => {
            if nonnegative(bound_generation)? != generation
                || nonnegative(bound_target)? != target_generation
                || bound_target_id != target.as_str()
                || bound_terminal.as_deref() != Some(terminal.as_str())
                || bound_incarnation.as_deref() != Some(incarnation.as_str())
            {
                return Ok(None);
            }
            (registered_at.map(|_| generation), Some(bound_harness))
        }
        // The seat's open binding is required (TRUST-POLICY A4): no binding,
        // no cooperative wake authority.
        None => return Ok(None),
    };
    Ok(Some(ReservedWakeAuthority::Cooperative {
        terminal: TerminalId::new(terminal.clone()),
        incarnation: incarnation.clone(),
        binding_generation,
        harness,
    }))
}

fn reason_labels(candidate: &WakeCandidate) -> Vec<String> {
    let mut reasons = Vec::with_capacity(3);
    if candidate.has_pending_invitation {
        reasons.push("pending_invitation".into());
    }
    if candidate.has_pending_receipt {
        reasons.push("pending_receipt".into());
    }
    if candidate.actionable_warning_seq.is_some()
        && !candidate.warning_offered_for_current_occupant()
    {
        reasons.push("actionable_warning".into());
    }
    reasons
}

pub fn reserve(
    context: &StoreContext,
    db: &mut Connection,
    instance: &str,
    candidate: &WakeCandidate,
    daemon_boot: uuid::Uuid,
    configured_minimum: u64,
) -> Result<Option<WakeReservation>, ApiError> {
    let Some(witness) = candidate.attention_witness.as_ref() else {
        return Ok(None);
    };
    if daemon_boot.is_nil() || configured_minimum < 30_000 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid wake configuration",
        ));
    }
    let committed=context.execute_decision(db,|tx|{
        type RowColumns = Option<(i64,i64,i64,i64,Option<String>,i64,String)>;
        let row:RowColumns=tx.query_row(
            "SELECT h.decision_seq,s.unavailability_episode,s.unavailability_open,s.generation,s.target_id,s.target_generation,h.host_boot FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1 AND s.instance_id=?2",
            params![candidate.seat.as_str(),instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(store_error)?;
        let Some((decision_seq,episode,open,generation,target,target_generation,boot))=row else {return Ok(None)};
        if !witness.valid_for(candidate,instance,nonnegative(decision_seq)?,nonnegative(episode)?,open!=0) {return Ok(None)}
        let attention=EffectiveSeatAttention {has_pending_invitation:witness.pending_invitation,
            has_pending_receipt:witness.pending_receipt,latest_warning_seq:witness.latest_warning_seq.map(|seq|i64::try_from(seq).map_err(|_|api_error(ErrorCode::StoreCorrupt,"warning sequence overflow"))).transpose()?,frontier:witness.frontier()};
        let current=load_candidate(tx,instance,&candidate.seat,&attention,decision_seq)?;
        if !current.has_actionable_work() || !current.continuity_resolved || current.reservation_id.is_some()
            || current.binding_generation!=candidate.binding_generation || current.binding_execution!=candidate.binding_execution
            || current.target!=candidate.target || current.reason_bits!=candidate.reason_bits
            || current.warning_offer!=candidate.warning_offer || current.attention_version!=candidate.attention_version
            || current.checkpoint_version!=candidate.checkpoint_version || current.retry_step!=candidate.retry_step
            || current.reservation_boot!=candidate.reservation_boot || current.last_reservation_id!=candidate.last_reservation_id
            || current.last_reservation_boot!=candidate.last_reservation_boot || current.minimum_delay_ms!=candidate.minimum_delay_ms
            || current.effective_delay_ms!=candidate.effective_delay_ms || current.last_outcome!=candidate.last_outcome
            || current.last_reserved_frontier!=candidate.last_reserved_frontier {return Ok(None)}
        let Some(target)=target.map(HostTargetId::new) else {return Ok(None)};
        let boot=HostBootId::new(boot);
        let host_epoch:i64=tx.query_row("SELECT host_epoch FROM host_instances WHERE id=?1",[instance],|r|r.get(0)).map_err(store_error)?;
        let Some(authority)=current_authority(tx,instance,&candidate.seat,&target,nonnegative(generation)?,nonnegative(target_generation)?,&boot,nonnegative(host_epoch)?)? else {return Ok(None)};
        Ok(Some((target,boot,nonnegative(host_epoch)?,nonnegative(target_generation)?,authority,current)))
    },|tx,at,prepared|{
        let Some((target,host_boot,host_epoch,target_generation,authority,current))=prepared else {return Ok(None)};
        // The scheduler has already enforced the retained monotonic guard.
        // This new reservation freezes the current daemon run's configured M.
        let minimum=configured_minimum;
        let step=if current.last_reservation_id.is_none(){0}else{current.retry_step.saturating_add(1).min(3)};
        let delay=minimum.max([30_000,60_000,120_000,300_000][step as usize]);
        at.monotonic.0.checked_add(5_000).ok_or_else(||api_error(ErrorCode::InvalidRequest,"wake lease overflow"))?;
        let attempt=WakeAttemptId::new(uuid::Uuid::new_v4().to_string());
        let frontier=witness.frontier();
        let sql_key=|key:Option<LogicalPublicationKey>| -> Result<(Option<i64>,Option<i64>),ApiError> {
            key.map(|key|Ok((Some(i64::try_from(key.decision_seq).map_err(|_|api_error(ErrorCode::StoreCorrupt,"wake frontier sequence overflow"))?),
                Some(i64::try_from(key.event_offset).map_err(|_|api_error(ErrorCode::StoreCorrupt,"wake frontier offset overflow"))?))))
                .unwrap_or(Ok((None,None)))
        };
        let (invitation_seq,invitation_offset)=sql_key(frontier.invitation)?;
        let (receipt_seq,receipt_offset)=sql_key(frontier.addressed_receipt)?;
        let (warning_seq,warning_offset)=sql_key(frontier.actionable_warning)?;
        let binding_generation=match &authority {ReservedWakeAuthority::Registered {binding_generation,..}|ReservedWakeAuthority::Cooperative {binding_generation:Some(binding_generation),..}=>Some(i64::try_from(*binding_generation).map_err(|_|api_error(ErrorCode::StoreCorrupt,"binding generation overflow"))?),_=>None};
        tx.execute("INSERT INTO wake_work(seat_id,reason_bits,binding_generation,retry_step,reservation_id,reservation_boot,reserved_at_utc,minimum_delay_ms,effective_delay_ms,last_reservation_id,last_reservation_boot,last_reserved_at_utc,last_invitation_seq,last_invitation_offset,last_receipt_seq,last_receipt_offset,last_warning_seq,last_warning_offset) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?5,?6,?7,?10,?11,?12,?13,?14,?15) ON CONFLICT(seat_id) DO UPDATE SET binding_generation=excluded.binding_generation,retry_step=excluded.retry_step,reservation_id=excluded.reservation_id,reservation_boot=excluded.reservation_boot,reserved_at_utc=excluded.reserved_at_utc,minimum_delay_ms=excluded.minimum_delay_ms,effective_delay_ms=excluded.effective_delay_ms,last_reservation_id=excluded.last_reservation_id,last_reservation_boot=excluded.last_reservation_boot,last_reserved_at_utc=excluded.last_reserved_at_utc,last_invitation_seq=excluded.last_invitation_seq,last_invitation_offset=excluded.last_invitation_offset,last_receipt_seq=excluded.last_receipt_seq,last_receipt_offset=excluded.last_receipt_offset,last_warning_seq=excluded.last_warning_seq,last_warning_offset=excluded.last_warning_offset",
            params![candidate.seat.as_str(),i64::try_from(current.reason_bits).map_err(|_|api_error(ErrorCode::StoreCorrupt,"wake reason overflow"))?,binding_generation,i64::from(step),attempt.as_str(),daemon_boot.to_string(),at.utc.0,i64::try_from(minimum).map_err(|_|api_error(ErrorCode::InvalidRequest,"wake minimum overflow"))?,i64::try_from(delay).map_err(|_|api_error(ErrorCode::InvalidRequest,"wake delay overflow"))?,invitation_seq,invitation_offset,receipt_seq,receipt_offset,warning_seq,warning_offset]).map_err(store_error)?;
        Ok(Some(WakeReservation {attempt,daemon_boot,seat:candidate.seat.clone(),reasons:reason_labels(&current),
            retained_effective_delay_ms:delay,lease_until:at.monotonic,retained_minimum_delay_ms:minimum,reserved_at_utc:at.utc,
            host_boot,host_epoch,target,target_generation,attention_witness:witness.clone(),authority}))
    })?;
    // A slow COMMIT must not consume the dispatcher lease before it starts.
    Ok(committed.map(|mut reservation| {
        reservation.lease_until =
            MonoInstant(context.clock().monotonic_now().0.saturating_add(5_000));
        reservation
    }))
}

/// The dispatcher calls this after its fresh host read and before submission.
/// The read transaction checks the committed attempt and the same attention
/// revision accepted at reservation, without repeating an unbounded scan.
pub fn validate_reservation(
    db: &Connection,
    instance: &str,
    reservation: &WakeReservation,
) -> Result<bool, ApiError> {
    type RowColumns = Option<(i64, i64, i64, String, i64, i64, Option<String>, i64)>;
    let row:RowColumns=db.query_row(
        "SELECT h.decision_seq,s.unavailability_episode,s.unavailability_open,h.host_boot,h.host_epoch,s.target_generation,s.target_id,s.generation FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1 AND s.instance_id=?2 AND s.state='resolved'",
        params![reservation.seat.as_str(),instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(store_error)?;
    let Some((
        seq,
        episode,
        open,
        host_boot,
        host_epoch,
        target_generation,
        target,
        seat_generation,
    )) = row
    else {
        return Ok(false);
    };
    if !reservation.attention_witness.valid_at(
        instance,
        &reservation.seat,
        nonnegative(seq)?,
        nonnegative(episode)?,
        open != 0,
    ) || host_boot != reservation.host_boot.as_str()
        || nonnegative(host_epoch)? != reservation.host_epoch
        || nonnegative(target_generation)? != reservation.target_generation
        || target.as_deref() != Some(reservation.target.as_str())
    {
        return Ok(false);
    }
    let attention = EffectiveSeatAttention {
        has_pending_invitation: reservation.attention_witness.pending_invitation,
        has_pending_receipt: reservation.attention_witness.pending_receipt,
        latest_warning_seq: reservation
            .attention_witness
            .latest_warning_seq
            .map(|n| {
                i64::try_from(n)
                    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "warning sequence overflow"))
            })
            .transpose()?,
        frontier: reservation.attention_witness.frontier(),
    };
    let candidate = load_candidate(db, instance, &reservation.seat, &attention, seq)?;
    if !candidate.continuity_resolved
        || !candidate.has_actionable_work()
        || candidate.reservation_id.as_ref() != Some(&reservation.attempt)
        || candidate
            .reservation_boot
            .as_ref()
            .is_none_or(|boot| boot.as_str() != reservation.daemon_boot.to_string())
    {
        return Ok(false);
    }
    // The seat generation, exactly as at reservation: a cooperative binding
    // may carry an older host epoch (the D2 reasoning) and so is not the
    // candidate's epoch-current binding.
    let current = current_authority(
        db,
        instance,
        &reservation.seat,
        &reservation.target,
        nonnegative(seat_generation)?,
        reservation.target_generation,
        &reservation.host_boot,
        reservation.host_epoch,
    )?;
    Ok(current.as_ref() == Some(&reservation.authority))
}

/// Settles one reservation. For `Refused` with a `refused_restore`, the same
/// fenced UPDATE (`reservation_id` + `reservation_boot` match) restores the
/// pre-reservation ladder row and returns whether it matched. A path that
/// cleared the reservation in between (host invalidation, registration loss)
/// matches 0 rows: no error, the durable step stays advanced by one.
pub fn complete(
    context: &StoreContext,
    db: &mut Connection,
    attempt: &WakeAttemptId,
    daemon_boot: &uuid::Uuid,
    outcome: WakeOutcome,
    refused_restore: Option<&PriorLadder>,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    complete_with_pokes(
        context,
        db,
        attempt,
        daemon_boot,
        outcome,
        refused_restore,
        budget,
        &[],
    )
}

/// [`complete`] for a poke (or a wake that carried the poke text): when the
/// reservation settles `submitted` in this transaction, `soft_poked_at` is set
/// on exactly `receipts`. A stale, foreign-boot or non-submitted completion
/// marks nothing, so an abandoned poke leaves its receipts to be poked again.
// Allowed: `complete`'s inputs plus the receipts the poke marks.
#[allow(clippy::too_many_arguments)]
pub fn complete_with_pokes(
    context: &StoreContext,
    db: &mut Connection,
    attempt: &WakeAttemptId,
    daemon_boot: &uuid::Uuid,
    outcome: WakeOutcome,
    refused_restore: Option<&PriorLadder>,
    budget: &CallBudget,
    receipts: &[PokeReceipt],
) -> Result<bool, ApiError> {
    if attempt.as_str().is_empty() || daemon_boot.is_nil() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "wake attempt and daemon boot required",
        ));
    }
    context.execute_budgeted_decision(db,budget,|tx|{
        type RowColumns = Option<(String,Option<String>,Option<i64>,i64,String)>;
        let row:RowColumns=tx.query_row(
            "SELECT w.seat_id,w.reservation_boot,w.binding_generation,s.generation,s.state FROM wake_work w JOIN seats s ON s.id=w.seat_id WHERE w.reservation_id=?1",
            [attempt.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(store_error)?;
        Ok(row)
    },|tx,at,row|{
        let Some((seat,reservation_boot,binding_generation,current_generation,state))=row else {return Ok(false)};
        let daemon_boot=daemon_boot.to_string();
        if reservation_boot.as_deref()!=Some(daemon_boot.as_str()) {return Ok(false)}
        let disposition=if state!="resolved" || binding_generation.is_some_and(|generation|generation!=current_generation) {
            "unsafe"
        } else {match outcome {WakeOutcome::Submitted=>"submitted",WakeOutcome::OutcomeUnknown=>"outcome_unknown",
            WakeOutcome::Unsafe|WakeOutcome::Refused(RefusalCause::Unsafe)=>"unsafe",
            WakeOutcome::Unavailable|WakeOutcome::Refused(RefusalCause::Unavailable)=>"unavailable",
            WakeOutcome::TimedOut|WakeOutcome::Refused(RefusalCause::TimedOut)=>"timed_out",
            WakeOutcome::Cancelled=>"cancelled"}};
        let restore=match (outcome,refused_restore) {(WakeOutcome::Refused(_),Some(prior))=>Some(prior),_=>None};
        let changed=match restore {
            None=>tx.execute("UPDATE wake_work SET reservation_id=NULL,reservation_boot=NULL,completed_at_utc=?1,last_outcome=?2 WHERE seat_id=?3 AND reservation_id=?4 AND reservation_boot=?5",
                params![at.utc.0,disposition,seat,attempt.as_str(),daemon_boot]).map_err(store_error)?,
            Some(prior)=>{
                let key=|key:Option<LogicalPublicationKey>| -> Result<(Option<i64>,Option<i64>),ApiError> {
                    key.map(|key|Ok((Some(i64::try_from(key.decision_seq).map_err(|_|api_error(ErrorCode::StoreCorrupt,"wake frontier sequence overflow"))?),
                        Some(i64::try_from(key.event_offset).map_err(|_|api_error(ErrorCode::StoreCorrupt,"wake frontier offset overflow"))?))))
                        .unwrap_or(Ok((None,None)))
                };
                let (invitation_seq,invitation_offset)=key(prior.last_reserved_frontier.invitation)?;
                let (receipt_seq,receipt_offset)=key(prior.last_reserved_frontier.addressed_receipt)?;
                let (warning_seq,warning_offset)=key(prior.last_reserved_frontier.actionable_warning)?;
                tx.execute("UPDATE wake_work SET reservation_id=NULL,reservation_boot=NULL,completed_at_utc=?1,last_outcome=?2,retry_step=?6,minimum_delay_ms=?7,effective_delay_ms=?8,last_reservation_id=?9,last_reservation_boot=?10,last_reserved_at_utc=?11,last_invitation_seq=?12,last_invitation_offset=?13,last_receipt_seq=?14,last_receipt_offset=?15,last_warning_seq=?16,last_warning_offset=?17 WHERE seat_id=?3 AND reservation_id=?4 AND reservation_boot=?5",
                    params![at.utc.0,disposition,seat,attempt.as_str(),daemon_boot,
                        i64::from(prior.retry_step),
                        i64::try_from(prior.minimum_delay_ms).map_err(|_|api_error(ErrorCode::InvalidRequest,"wake minimum overflow"))?,
                        i64::try_from(prior.effective_delay_ms).map_err(|_|api_error(ErrorCode::InvalidRequest,"wake delay overflow"))?,
                        prior.last_reservation_id.as_ref().map(|id|id.as_str().to_owned()),
                        prior.last_reservation_boot.as_ref().map(|boot|boot.as_str().to_owned()),
                        prior.last_reserved_at_utc.map(|at|at.0),
                        invitation_seq,invitation_offset,receipt_seq,receipt_offset,warning_seq,warning_offset]).map_err(store_error)?
            }
        };
        if changed==1 && disposition=="submitted" {
            poke::mark_soft_poked(tx,receipts,at.utc)?;
        }
        Ok(restore.is_some() && changed==1)
    })
}

/// Reserves the seat's single wake slot for a soft-deadline poke (spec §10):
/// the same reservation id/boot/lease columns as a wake, refused while any
/// reservation is active. Unlike `reserve` it needs no attention advance and
/// leaves every retry/frontier column alone, so a poke never shifts the wake
/// guard's history. The receipts still due are rechecked inside the
/// reserving transaction; the reservation's attention witness asserts only
/// the pending receipt the poke was selected for (never a complete scan).
pub fn reserve_poke(
    context: &StoreContext,
    db: &mut Connection,
    instance: &str,
    due: &PokeDue,
    daemon_boot: uuid::Uuid,
    settings: &SummarySettings,
) -> Result<Option<PokeReservation>, ApiError> {
    if daemon_boot.is_nil() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid wake configuration",
        ));
    }
    let seat = &due.seat;
    let committed=context.execute_decision(db,|tx|{
        type RowColumns = Option<(i64,i64,i64,i64,Option<String>,i64,String,i64)>;
        let row:RowColumns=tx.query_row(
            "SELECT h.decision_seq,s.unavailability_episode,s.unavailability_open,s.generation,s.target_id,s.target_generation,h.host_boot,h.host_epoch FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1 AND s.instance_id=?2",
            params![seat.as_str(),instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(store_error)?;
        let Some((decision_seq,episode,open,generation,target,target_generation,boot,host_epoch))=row else {return Ok(None)};
        let Some(target)=target.map(HostTargetId::new) else {return Ok(None)};
        // Only the receipts still due now are covered, never a stale selection.
        let now=context.clock().utc_now().0;
        let Some(current_due)=poke::due_pokes_for_seat(tx,seat,now,settings)? else {return Ok(None)};
        let receipts:Vec<PokeReceipt>=due.receipts.iter().filter(|r|current_due.receipts.iter().any(|c|c.message==r.message&&c.source==r.source)).cloned().collect();
        if receipts.is_empty() {return Ok(None)}
        let attention=EffectiveSeatAttention {has_pending_invitation:false,has_pending_receipt:true,latest_warning_seq:None,frontier:LogicalAttentionFrontier::default()};
        let current=load_candidate(tx,instance,seat,&attention,decision_seq)?;
        if current.effectively_retired || !current.continuity_resolved || current.reservation_id.is_some() {return Ok(None)}
        let boot=HostBootId::new(boot);
        let Some(authority)=current_authority(tx,instance,seat,&target,nonnegative(generation)?,nonnegative(target_generation)?,&boot,nonnegative(host_epoch)?)? else {return Ok(None)};
        let witness=WakeAttentionWitness::from_complete(instance.into(),seat.clone(),nonnegative(decision_seq)?,false,true,None,nonnegative(episode)?,open!=0,LogicalAttentionFrontier::default());
        Ok(Some((target,boot,nonnegative(host_epoch)?,nonnegative(target_generation)?,authority,current,witness,receipts)))
    },|tx,at,prepared|{
        let Some((target,host_boot,host_epoch,target_generation,authority,current,witness,receipts))=prepared else {return Ok(None)};
        at.monotonic.0.checked_add(5_000).ok_or_else(||api_error(ErrorCode::InvalidRequest,"wake lease overflow"))?;
        let attempt=WakeAttemptId::new(uuid::Uuid::new_v4().to_string());
        let binding_generation=match &authority {ReservedWakeAuthority::Registered {binding_generation,..}|ReservedWakeAuthority::Cooperative {binding_generation:Some(binding_generation),..}=>Some(i64::try_from(*binding_generation).map_err(|_|api_error(ErrorCode::StoreCorrupt,"binding generation overflow"))?),_=>None};
        tx.execute("INSERT INTO wake_work(seat_id,binding_generation,reservation_id,reservation_boot,reserved_at_utc) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(seat_id) DO UPDATE SET binding_generation=excluded.binding_generation,reservation_id=excluded.reservation_id,reservation_boot=excluded.reservation_boot,reserved_at_utc=excluded.reserved_at_utc",
            params![seat.as_str(),binding_generation,attempt.as_str(),daemon_boot.to_string(),at.utc.0]).map_err(store_error)?;
        Ok(Some(PokeReservation {reservation:WakeReservation {attempt,daemon_boot,seat:seat.clone(),reasons:vec!["soft_deadline".into()],
            retained_effective_delay_ms:current.effective_delay_ms,lease_until:at.monotonic,retained_minimum_delay_ms:current.minimum_delay_ms,reserved_at_utc:at.utc,
            host_boot,host_epoch,target,target_generation,attention_witness:witness,authority},receipts}))
    })?;
    // A slow COMMIT must not consume the dispatcher lease before it starts.
    Ok(committed.map(|mut reserved| {
        reserved.reservation.lease_until =
            MonoInstant(context.clock().monotonic_now().0.saturating_add(5_000));
        reserved
    }))
}

/// Settle only the exact reservation left by a different daemon boot. The
/// elected boot comes from this store's owner settings; request fields merely
/// identify a row to compare. No eligibility or prompt follows this write.
pub fn recover_abandoned(
    context: &StoreContext,
    db: &mut Connection,
    instance: &str,
    elected_boot: Option<uuid::Uuid>,
    request: &WakeRecoveryRequest,
) -> Result<WakeRecoveryOutcome, ApiError> {
    let Some(current_boot) = elected_boot else {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "wake recovery requires elected daemon boot",
        ));
    };
    if current_boot.is_nil()
        || request.elected_boot != current_boot
        || request.prior_daemon_boot.is_nil()
        || request.prior_daemon_boot == current_boot
        || request.instance != instance
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid wake recovery owner fence",
        ));
    }
    context.execute_decision(db, |tx| {
        type RowColumns = Option<(Option<String>, Option<String>, Option<String>, Option<String>)>;
        let row: RowColumns = tx.query_row(
            "SELECT w.reservation_id,w.reservation_boot,w.last_reservation_id,w.last_reservation_boot FROM wake_work w JOIN seats s ON s.id=w.seat_id WHERE w.seat_id=?1 AND s.instance_id=?2",
            params![request.seat.as_str(),instance],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional().map_err(store_error)?;
        Ok(row)
    }, |tx, at, row| {
        let Some((active, boot, last, last_boot)) = row else { return Ok(WakeRecoveryOutcome::Stale) };
        let prior = request.prior_daemon_boot.to_string();
        if active.as_deref() == Some(request.attempt.as_str()) && boot.as_deref() == Some(prior.as_str()) {
            let changed = tx.execute(
                "UPDATE wake_work SET reservation_id=NULL,reservation_boot=NULL,completed_at_utc=?1,last_outcome='outcome_unknown' WHERE seat_id=?2 AND reservation_id=?3 AND reservation_boot=?4",
                params![at.utc.0,request.seat.as_str(),request.attempt.as_str(),prior],
            ).map_err(store_error)?;
            return Ok(if changed == 1 { WakeRecoveryOutcome::Recovered } else { WakeRecoveryOutcome::Stale });
        }
        if active.is_none() && last.as_deref() == Some(request.attempt.as_str()) && last_boot.as_deref() == Some(prior.as_str()) {
            return Ok(WakeRecoveryOutcome::AlreadySettled);
        }
        Ok(WakeRecoveryOutcome::Stale)
    })
}

/// The retained-window table drives this walk through its primary key. Cost
/// follows retained windows, rather than all resolved or historical seats.
pub(crate) const BATCH_SEATS_SQL: &str = "SELECT b.seat_id FROM wake_batches b CROSS JOIN seats s ON s.id=b.seat_id WHERE s.instance_id=?1 AND b.seat_id>?2 ORDER BY b.seat_id LIMIT ?3";

fn receipt_attention_at(
    db: &Connection,
    seat: &str,
    receipt: &effective::EffectiveReceipt,
    key: (i64, i64),
) -> Result<i64, ApiError> {
    if receipt
        .decision_seq
        .is_some_and(|original| key.0 > original)
    {
        let release: Option<i64> = db.query_row(
            "SELECT ended_at FROM catch_up WHERE seat_id=?1 AND thread_id=?2 AND release_seq=?3",
            params![seat,receipt.thread_id,key.0], |r| r.get(0)).optional().map_err(store_error)?.flatten();
        return release.ok_or_else(|| {
            api_error(
                ErrorCode::StoreCorrupt,
                "released attention timestamp missing",
            )
        });
    }
    Ok(receipt.decision_at)
}

/// Oldest-first indexed probes keep a saturated newest-first digest from
/// anchoring the batch to newer arrivals. Every source retains the same row cap.
fn oldest_ordinary_at(db: &Connection, seat: &str, through: i64) -> Result<Option<i64>, ApiError> {
    let cap = super::attention::WINDOW as i64;
    let mut earliest = None::<i64>;
    let mut invitations = db.prepare(
        "SELECT i.created_at FROM (SELECT invitation_id FROM digest_pending_invitations INDEXED BY digest_pending_invitations_seat WHERE seat_id=?1 AND created_decision_seq<=?2 ORDER BY created_decision_seq,ordinal LIMIT ?3) w JOIN invitations i ON i.id=w.invitation_id WHERE i.state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id)").map_err(store_error)?;
    let dates = invitations
        .query_map(params![seat, through, cap], |r| r.get::<_, i64>(0))
        .map_err(store_error)?;
    for date in dates {
        let date = date.map_err(store_error)?;
        earliest = Some(earliest.map_or(date, |prior| prior.min(date)));
    }
    let mut hold = super::catch_up::HoldCache::new(seat);
    for (source, sql) in [
        (
            effective::ReceiptSource::Physical,
            "SELECT message_id FROM receipts INDEXED BY receipts_required_seat_pending WHERE seat_id=?1 AND state='pending' AND ack_required=1 ORDER BY ordinal LIMIT ?2",
        ),
        (
            effective::ReceiptSource::Manifest,
            "SELECT sm.message_id FROM (SELECT preparation_id FROM digest_pending_manifest_receipts INDEXED BY digest_pending_manifest_receipts_seat WHERE seat_id=?1 AND decision_seq>0 ORDER BY decision_seq,ordinal LIMIT ?2) w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id",
        ),
        (
            effective::ReceiptSource::Manifest,
            "SELECT sm.message_id FROM (SELECT preparation_id FROM digest_pending_manifest_receipts INDEXED BY digest_pending_manifest_receipts_seat WHERE seat_id=?1 AND decision_seq IS NULL ORDER BY ordinal LIMIT ?2) w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id",
        ),
    ] {
        let mut statement = db.prepare(sql).map_err(store_error)?;
        let messages = statement
            .query_map(params![seat, cap], |r| r.get::<_, String>(0))
            .map_err(store_error)?;
        for message in messages {
            let message = message.map_err(store_error)?;
            let Some(receipt) = effective::effective_receipt(db, &message, seat)? else {
                continue;
            };
            if receipt.source != source
                || receipt.state != effective::EffectiveReceiptState::Pending
            {
                continue;
            }
            let Some(seq) = receipt.decision_seq else {
                continue;
            };
            let Some(key) =
                hold.attention_key(db, &receipt.thread_id, &message, receipt.sequence, (seq, 0))?
            else {
                continue;
            };
            let at = receipt_attention_at(db, seat, &receipt, key)?;
            earliest = Some(earliest.map_or(at, |prior| prior.min(at)));
        }
    }
    Ok(earliest)
}

/// Decide the initial ordinary window against canonical bounded attention.
/// Publication timestamps make discovery after a host outage mature promptly.
pub fn batch_window(
    context: &StoreContext,
    db: &mut Connection,
    instance: &str,
    candidate: &WakeCandidate,
    delay_ms: u64,
) -> Result<Option<(crate::protocol::time::UtcMillis, u64)>, ApiError> {
    if delay_ms == 0 {
        return Ok(None);
    }
    context.execute_decision(db, |tx| {
        let live: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2 AND state!='retired')",
            params![candidate.seat.as_str(), instance], |r| r.get(0)).map_err(store_error)?;
        if !live { return Ok((None, false)); }
        let scan = super::attention::wake_seat_attention(tx, candidate.seat.as_str())?;
        let invitations = super::attention::pending_invitations(tx, candidate.seat.as_str(), None, scan.decision_seq)?;
        let pending = super::attention::pending_receipts(tx, candidate.seat.as_str(), None)?;
        let mut first = None::<i64>;
        let mut urgent = scan.attention.latest_warning_seq.is_some()
            && !candidate.warning_offered_for_current_occupant();
        let now = context.clock().utc_now().0;
        for item in invitations.items {
            let (created, due): (i64, Option<i64>) = tx.query_row(
                "SELECT created_at,deadline_at FROM invitations WHERE id=?1", [&item.id],
                |r| Ok((r.get(0)?,r.get(1)?))).map_err(store_error)?;
            first = Some(first.map_or(created, |prior| prior.min(created)));
            urgent |= due.is_some_and(|at| at <= now);
        }
        for item in pending.items {
            if let Some(receipt) = effective::effective_receipt(tx, &item.id, candidate.seat.as_str())? {
                let at = receipt_attention_at(tx, candidate.seat.as_str(), &receipt, item.key)?;
                first = Some(first.map_or(at, |prior| prior.min(at)));
                urgent |= super::receipts::effective_deadline(tx, &receipt)?.is_some_and(|at| at <= now);
            }
        }
        let existing: Option<i64> = tx.query_row("SELECT deadline_at FROM wake_batches WHERE seat_id=?1",
            [candidate.seat.as_str()], |r| r.get(0)).optional().map_err(store_error)?;
        if let Some(first_at) = first
            && existing.is_none()
            && let Some(oldest) = oldest_ordinary_at(tx, candidate.seat.as_str(), scan.decision_seq)? {
            first = Some(first_at.min(oldest));
        }
        Ok((first, urgent))
    }, |tx, _, (first, urgent)| {
        let Some(first) = first else {
            tx.execute("DELETE FROM wake_batches WHERE seat_id=?1", [candidate.seat.as_str()]).map_err(store_error)?;
            return Ok(None);
        };
        let deadline = retain_batch_deadline(tx, candidate.seat.as_str(), first, delay_ms)?;
        Ok((!urgent).then_some((deadline, delay_ms)))
    })
}

/// Retain the first deadline without sliding it on later attention. The caller
/// has already classified canonical ordinary work inside this transaction.
fn retain_batch_deadline(
    db: &Connection,
    seat: &str,
    first_at: i64,
    delay_ms: u64,
) -> Result<crate::protocol::time::UtcMillis, ApiError> {
    let existing: Option<i64> = db
        .query_row(
            "SELECT deadline_at FROM wake_batches WHERE seat_id=?1",
            [seat],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if let Some(deadline) = existing {
        return Ok(crate::protocol::time::UtcMillis(deadline));
    }
    let deadline = first_at.saturating_add(delay_ms as i64);
    db.execute(
        "INSERT INTO wake_batches(seat_id,deadline_at) VALUES (?1,?2)",
        params![seat, deadline],
    )
    .map_err(store_error)?;
    Ok(crate::protocol::time::UtcMillis(deadline))
}

#[cfg(test)]
mod batch_tests {
    use super::*;

    #[test]
    fn batch_deadline_retains_first_arrival_through_reopen_and_clear() {
        let path = std::env::temp_dir().join(format!("wake-batch-{}.sqlite", uuid::Uuid::new_v4()));
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE wake_batches(seat_id TEXT PRIMARY KEY, deadline_at INTEGER NOT NULL);",
        )
        .unwrap();
        assert_eq!(
            retain_batch_deadline(&db, "seat", 100, 30_000).unwrap().0,
            30_100
        );
        assert_eq!(
            retain_batch_deadline(&db, "seat", 29_000, 30_000)
                .unwrap()
                .0,
            30_100
        );
        drop(db);
        let db = Connection::open(&path).unwrap();
        assert_eq!(
            retain_batch_deadline(&db, "seat", 99_000, 30_000)
                .unwrap()
                .0,
            30_100
        );
        db.execute("DELETE FROM wake_batches WHERE seat_id='seat'", [])
            .unwrap();
        assert_eq!(
            retain_batch_deadline(&db, "seat", 99_000, 30_000)
                .unwrap()
                .0,
            129_000
        );
        assert_eq!(retain_batch_deadline(&db, "zero", 100, 0).unwrap().0, 100);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
