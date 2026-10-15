//! Invitations and consent.

use super::{
    bump_member_directory, cooperative_payload_hash, decide_accountable, guard_pair_member,
    joined_in_thread, validate_actor,
};
use crate::ports::TimeBasis;
use crate::protocol::authority::{MutationPermit, ObligationRef, OperatorActor};
use crate::protocol::commands::{Accept, AcceptRequired, Invite, OperatorOrphanInvite};
use crate::protocol::ids::{InvitationId, SeatId, ThreadId, prefix};
use crate::protocol::results::{ApiError, CommandResult, ErrorCode};
use crate::protocol::time::{CallBudget, UtcMillis};
use crate::store::connection::{StoreContext, api_error, store_error};
use crate::store::schema::{self, EventInput};
use rusqlite::{Connection, OptionalExtension, params};

/// Prefer work the caller can still settle over a newer accepted invitation.
/// The fallback preserves idempotent acceptance and terminal-state diagnostics.
/// Permit preparation and acceptance must select the same exact obligation.
pub(crate) fn current_invitation_for_accept(
    conn: &Connection,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<Option<InvitationId>, ApiError> {
    let pending: Option<String> = conn.query_row(
        "SELECT i.id FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending'
         AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id)
         AND NOT EXISTS (SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=i.id)
         ORDER BY i.episode DESC LIMIT 1",
        params![thread.as_str(), seat.as_str()], |r| r.get(0),
    ).optional().map_err(store_error)?;
    if let Some(id) = pending {
        return Ok(Some(InvitationId::new(id)));
    }
    let latest: Option<String> = conn
        .query_row(
            "SELECT i.id FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2
         AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id)
         ORDER BY i.episode DESC LIMIT 1",
            params![thread.as_str(), seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(latest.map(InvitationId::new))
}

pub fn accept(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &Accept,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("accept", command)?;
    let invitation = current_invitation_for_accept(conn, &command.thread, &caller)?
        .ok_or_else(|| api_error(ErrorCode::NotFound,format!("current invitation missing: seat {} has no invitation in thread {}; pass a thread ID listed by herdr-threads inbox",caller.as_str(),command.thread.as_str())))?;
    let obligation = ObligationRef::Invitation(invitation.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            guard_pair_member(tx, &command.thread, &caller)?;
            validate_actor(tx, &caller, target)?;
            if let Some(current) =
                crate::store::service_substrate::current_requirement(tx, &command.thread, &caller)?
                && current.invitation == invitation
                && current.state == crate::protocol::service::RequirementState::Pending
            {
                return Err(api_error(
                    ErrorCode::StaleRequirementAcceptance,
                    format!(
                        "reread thread participants and accept the current requirement revision: {}",
                        serde_json::to_string(&current)
                            .map_err(|e| api_error(ErrorCode::StoreCorrupt, e.to_string()))?
                    ),
                ));
            }
            let invitation_seat: Option<String> = tx
                .query_row(
                    "SELECT seat_id FROM invitations WHERE id=?1 AND thread_id=?2",
                    params![invitation.as_str(), command.thread.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            if invitation_seat.as_deref() != Some(caller.as_str()) {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "invitation is not addressed to caller",
                ));
            }
            Ok(())
        },
        |tx, decision| {
            let actor = decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let (thread, state): (String, String) = tx
                .query_row(
                    "SELECT thread_id,state FROM invitations WHERE id=?1",
                    [invitation.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(store_error)?;
            let rejected: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM invitation_rejections WHERE invitation_id=?1)",
                    [invitation.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if rejected {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "invitation was rejected; a fresh invitation is required",
                ));
            }
            if state == "accepted" {
                return Ok(CommandResult::Accepted(invitation.clone().into()));
            }
            if state != "pending" {
                return Err(api_error(ErrorCode::Conflict, "invitation is terminal"));
            }
            let cancelled: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM invitation_cancellations WHERE invitation_id=?1)",
                    [invitation.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if cancelled {
                return Err(api_error(ErrorCode::Conflict, "invitation was cancelled"));
            }
            let thread = ThreadId::new(thread);
            let already_joined = joined_in_thread(tx, &thread, &caller)?;
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            // Capture the current episode while the invitation/membership are
            // still pending/invited. A generic acceptance keeps its head policy.
            let direct_floor = if already_joined {
                None
            } else {
                crate::store::lazy_delivery::direct_acceptance_floor(
                    tx,
                    &instance,
                    &caller,
                    &thread,
                    &invitation,
                )?
            };
            schema::record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, decision.utc)?;
            let generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let observation = actor.observation(decision.utc.0);
            tx.execute("UPDATE invitations SET state='accepted', accepted_at=?1,accepted_actor_seat_id=?2,accepted_generation=?3,accepted_observation=?4 WHERE id=?5",
                params![decision.utc.0,caller.as_str(),generation,observation,invitation.as_str()]).map_err(store_error)?;
            schema::clear_warning_condition_for_invitation(tx, invitation.as_str(), decision.utc)?;
            let event_seq = schema::next_decision_seq(tx, &instance)?;
            // This invitation needs its own acceptance event even when another
            // invitation already joined the seat. The join interval belongs to
            // the first transition into Joined and must keep that frontier.
            if !already_joined {
                let episode: i64 = tx
                    .query_row(
                        "SELECT episode FROM invitations WHERE id=?1",
                        [invitation.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                tx.execute("UPDATE memberships SET state='joined',voluntary_state='joined',episode=?1,joined_at=?2,left_at=NULL WHERE thread_id=?3 AND seat_id=?4",
                    params![episode,decision.utc.0,thread.as_str(),caller.as_str()]).map_err(store_error)?;
                tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,?3,?4)",
                    params![thread.as_str(),caller.as_str(),episode,event_seq as i64]).map_err(store_error)?;
                match direct_floor {
                    Some(floor) => {
                        crate::store::read_cursors::init_for_direct_accept(
                            tx, &caller, &thread, floor,
                        )?;
                    }
                    None => {
                        crate::store::read_cursors::init_for_join(tx, &caller, &thread)?;
                    }
                }
            }
            let payload = serde_json::json!({"action":"accept", "seat":caller.as_str(), "invitation":invitation.as_str()}).to_string();
            schema::append_event_once_with_decision_seq(
                tx,
                EventInput {
                    thread: &thread,
                    key: &format!("accept:{}", invitation.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: Some(&invitation),
                },
                event_seq,
            )?;
            schema::bump_membership_revision(tx, &thread)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &caller)?;
            schema::bump_filter_revision(tx, &instance, "inbox", caller.as_str())?;
            Ok(CommandResult::Accepted(invitation.clone().into()))
        },
    )
}

/// Explicit native consent to the exact requirement revision shown to the
/// caller. An ordinary acceptance can never create this provenance.
/// Reject an exact invitation in the same canonical accountable transaction as acceptance.
pub fn reject(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &crate::protocol::commands::Reject,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    crate::protocol::commands::validate_rejection_reason(&command.reason)
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("reject", command)?;
    let obligation = ObligationRef::Invitation(command.invitation.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            validate_actor(tx, &caller, command.claim.target.as_str())?;
            let recipient: Option<String> = tx
                .query_row(
                    "SELECT seat_id FROM invitations WHERE id=?1 AND thread_id=?2",
                    params![command.invitation.as_str(), command.thread.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            if recipient.as_deref() != Some(caller.as_str()) {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "invitation is not addressed to caller in this thread",
                ));
            }
            if let Some(required) =
                crate::store::service_substrate::current_requirement(tx, &command.thread, &caller)?
                && required.invitation == command.invitation
                && matches!(
                    required.state,
                    crate::protocol::service::RequirementState::Pending
                        | crate::protocol::service::RequirementState::Accepted
                )
            {
                return Err(api_error(
                    ErrorCode::MembershipRequired,
                    format!(
                        "invitation {} is required by {}; ask that service owner to release requirement {} before rejecting; reread thread participants afterward",
                        command.invitation.as_str(),
                        required.issuer.as_str(),
                        required.requirement.as_str(),
                    ),
                ));
            }
            Ok(())
        },
        |tx, decision| {
            let actor = decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            type Saved = (String, i64, String, i64, String);
            let saved: Option<Saved> = tx.query_row(
                "SELECT actor_seat_id,generation,observation,rejected_at,reason FROM invitation_rejections WHERE invitation_id=?1",
                [command.invitation.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
            ).optional().map_err(store_error)?;
            if let Some((seat, generation, observation, at, reason)) = saved {
                if reason != command.reason {
                    return Err(api_error(
                        ErrorCode::Conflict,
                        "invitation already rejected with a different reason",
                    ));
                }
                return Ok(CommandResult::Rejected(
                    crate::protocol::results::InvitationRejection {
                        invitation: command.invitation.clone(),
                        actor: SeatId::new(seat),
                        generation: generation as u64,
                        observation,
                        rejected_at: UtcMillis(at),
                        reason,
                    },
                ));
            }
            let pending: bool = tx.query_row(
                "SELECT state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations WHERE invitation_id=?1) AND NOT EXISTS(SELECT 1 FROM invitation_rejections WHERE invitation_id=?1) FROM invitations WHERE id=?1",
                [command.invitation.as_str()], |r| r.get(0),
            ).map_err(store_error)?;
            if !pending {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "invitation is terminal or cancelled",
                ));
            }
            // Lateness is retained, never described as expiration or acceptance.
            schema::record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, decision.utc)?;
            let generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let observation = actor.observation(decision.utc.0);
            tx.execute("INSERT INTO invitation_rejections(invitation_id,reason,rejected_at,actor_seat_id,generation,observation) VALUES (?1,?2,?3,?4,?5,?6)",
                params![command.invitation.as_str(),command.reason,decision.utc.0,caller.as_str(),generation,observation]).map_err(store_error)?;
            schema::clear_warning_condition_for_invitation(
                tx,
                command.invitation.as_str(),
                decision.utc,
            )?;
            // Only an invited voluntary marker is cleared; no join or leave interval is fabricated.
            // Restore a prior left state only with its recorded closed interval and leave event.
            let prior_left: Option<i64> = tx.query_row(
                "SELECT m.decision_at FROM membership_intervals mi JOIN messages m ON m.thread_id=mi.thread_id AND m.decision_seq=mi.left_seq WHERE mi.thread_id=?1 AND mi.seat_id=?2 AND mi.left_seq IS NOT NULL AND json_extract(m.event_json,'$.action')='leave' AND json_extract(m.event_json,'$.seat')=?2 ORDER BY mi.episode DESC LIMIT 1",
                params![command.thread.as_str(),caller.as_str()], |r| r.get(0),
            ).optional().map_err(store_error)?;
            tx.execute("UPDATE memberships SET voluntary_state=?1,left_at=?2 WHERE thread_id=?3 AND seat_id=?4 AND voluntary_state='invited' AND episode=(SELECT episode FROM invitations WHERE id=?5) AND NOT EXISTS(SELECT 1 FROM invitations i WHERE i.thread_id=?3 AND i.seat_id=?4 AND i.state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=i.id))",
                params![if prior_left.is_some() { "left" } else { "absent" },prior_left,command.thread.as_str(),caller.as_str(),command.invitation.as_str()]).map_err(store_error)?;
            let payload = serde_json::json!({"action":"reject","seat":caller.as_str(),"invitation":command.invitation.as_str(),"reason":command.reason}).to_string();
            let (warning, inserted) = schema::append_attributed_event_once(
                tx,
                EventInput {
                    thread: &command.thread,
                    key: &format!("reject:{}", command.invitation.as_str()),
                    kind: "warn",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: Some(&command.invitation),
                },
                crate::protocol::service::EventAuthor::Native(caller.clone()),
            )?;
            if inserted {
                schema::enqueue_invitation_rejection_notice(
                    tx,
                    warning.as_str(),
                    command.thread.as_str(),
                    caller.as_str(),
                )?;
            }
            schema::bump_membership_revision(tx, &command.thread)?;
            schema::bump_filter_revision(tx, &command.claim.instance, "directory", "all")?;
            bump_member_directory(tx, &command.claim.instance, &caller)?;
            schema::bump_filter_revision(tx, &command.claim.instance, "inbox", caller.as_str())?;
            Ok(CommandResult::Rejected(
                crate::protocol::results::InvitationRejection {
                    invitation: command.invitation.clone(),
                    actor: caller.clone(),
                    generation: generation as u64,
                    observation,
                    rejected_at: decision.utc,
                    reason: command.reason.clone(),
                },
            ))
        },
    )
}

pub fn accept_required(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &AcceptRequired,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("accept_required", command)?;
    let obligation = ObligationRef::Invitation(command.invitation.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            guard_pair_member(tx, &command.thread, &caller)?;
            validate_actor(tx, &caller, command.claim.target.as_str())?;
            let current =
                crate::store::service_substrate::current_requirement(tx, &command.thread, &caller)?
                    .ok_or_else(|| {
                        api_error(ErrorCode::NotFound, "effective requirement missing")
                    })?;
            let repeat_accepted = current.state
                == crate::protocol::service::RequirementState::Accepted
                && current.requirement == command.requirement
                && current.invitation == command.invitation
                && current.thread == command.thread
                && current.seat == caller
                && command.claim.seat == caller
                && (command.expected_revision == current.revision
                    || command.expected_revision.checked_add(1) == Some(current.revision));
            if !repeat_accepted {
                command.check_current(&current).map_err(|_| {
                    api_error(
                        ErrorCode::StaleRequirementAcceptance,
                        format!(
                            "reread thread participants and accept the current requirement revision: {}",
                            serde_json::to_string(&current)
                                .unwrap_or_else(|_| "current requirement unavailable".into())
                        ),
                    )
                })?;
            }
            Ok(())
        },
        |tx, decision| {
            if let Some(current) =
                crate::store::service_substrate::current_requirement(tx, &command.thread, &caller)?
                && current.state == crate::protocol::service::RequirementState::Accepted
            {
                return Ok(CommandResult::RequiredAccepted(current));
            }
            let actor = decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let observation = actor.observation(decision.utc.0);
            schema::record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, decision.utc)?;
            tx.execute("UPDATE invitations SET state='accepted',accepted_at=?1,accepted_actor_seat_id=?2,accepted_generation=?3,accepted_observation=?4 WHERE id=?5 AND state='pending'",
                params![decision.utc.0,caller.as_str(),generation,observation,command.invitation.as_str()]).map_err(store_error)?;
            schema::clear_warning_condition_for_invitation(
                tx,
                command.invitation.as_str(),
                decision.utc,
            )?;
            tx.execute("UPDATE requirement_episodes SET state='accepted',revision=revision+1,accepted_at=?1,accepted_by_seat_id=?2,accepted_generation=?3,accepted_observation=?4 WHERE id=?5 AND state='pending' AND revision=?6",
                params![decision.utc.0,caller.as_str(),generation,observation,command.requirement.as_str(),command.expected_revision as i64]).map_err(store_error)?;
            let joined = joined_in_thread(tx, &command.thread, &caller)?;
            if !joined {
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM seats WHERE id=?1",
                        [caller.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                let seq = schema::next_decision_seq(tx, &instance)?;
                let episode: i64 = tx
                    .query_row(
                        "SELECT episode FROM invitations WHERE id=?1",
                        [command.invitation.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                tx.execute("UPDATE memberships SET state='joined',voluntary_state='joined',episode=?1,joined_at=?2,left_at=NULL WHERE thread_id=?3 AND seat_id=?4",
                    params![episode,decision.utc.0,command.thread.as_str(),caller.as_str()]).map_err(store_error)?;
                tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,?3,?4)",
                    params![command.thread.as_str(),caller.as_str(),episode,seq as i64]).map_err(store_error)?;
                crate::store::read_cursors::init_for_join(tx, &caller, &command.thread)?;
            }
            let payload = serde_json::json!({"action":"accept_required","seat":caller.as_str(),
                "requirement":command.requirement.as_str(),"invitation":command.invitation.as_str()}).to_string();
            schema::append_attributed_event_once(
                tx,
                EventInput {
                    thread: &command.thread,
                    key: &format!("accept_required:{}", command.requirement.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: Some(&command.invitation),
                },
                crate::protocol::service::EventAuthor::Native(caller.clone()),
            )?;
            schema::bump_membership_revision(tx, &command.thread)?;
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &caller)?;
            schema::bump_filter_revision(tx, &instance, "inbox", caller.as_str())?;
            let current =
                crate::store::service_substrate::current_requirement(tx, &command.thread, &caller)?
                    .ok_or_else(|| {
                        api_error(ErrorCode::StoreCorrupt, "accepted requirement disappeared")
                    })?;
            Ok(CommandResult::RequiredAccepted(current))
        },
    )
}

/// Invite a nonretired seat. A pending episode is immutable, including its
/// deadline, even when the caller requests a different duration later.
pub fn invite(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &Invite,
    mut permit: MutationPermit,
    installation_default_ms: Option<u64>,
) -> Result<CommandResult, ApiError> {
    if command.confirmation == Some(crate::protocol::commands::InvitationConfirmation::Lazy)
        && command.deadline_millis.is_some()
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "lazy invitation conflicts with deadline",
        ));
    }
    if command.confirmation == Some(crate::protocol::commands::InvitationConfirmation::RequireAck)
        && command.claim.harness == crate::protocol::authority::Harness::Human
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "human invitation cannot require agent confirmation",
        ));
    }
    let confirmation = command.confirmation.map(|mode| match mode {
        crate::protocol::commands::InvitationConfirmation::Lazy => "lazy",
        crate::protocol::commands::InvitationConfirmation::RequireAck => "require_ack",
    });
    let timed = confirmation != Some("lazy");
    let duration = command
        .deadline_millis
        .or(installation_default_ms)
        .unwrap_or(300_000);
    let duration = i64::try_from(duration)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invitation duration too large"))?;
    if duration <= 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invitation duration must be positive",
        ));
    }
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("invite", command)?;
    let obligation = ObligationRef::Control(command.thread.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            guard_pair_member(tx, &command.thread, &command.seat)?;
            let instance = validate_actor(tx, &caller, target)?;
            let joined: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM memberships m JOIN seats s ON s.id=m.seat_id \
                 WHERE m.thread_id=?1 AND m.seat_id=?2 AND m.state='joined' AND s.state!='retired')",
                params![command.thread.as_str(), caller.as_str()], |r| r.get(0),
            ).map_err(store_error)?;
            if !joined {
                return Err(api_error(ErrorCode::Unauthorized, "caller is not joined"));
            }
            let target_instance: Option<String> = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1 AND state!='retired'",
                    [command.seat.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            if target_instance.as_deref() != Some(instance.as_str()) {
                return Err(api_error(
                    ErrorCode::TargetUnresolved,
                    "invite target missing, retired, or outside instance",
                ));
            }
            Ok(())
        },
        |tx, decision| {
            decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let prior: Option<(i64, String)> = tx
                .query_row(
                    "SELECT episode, state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                    params![command.thread.as_str(), command.seat.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            if let Some((_, state)) = &prior
                && state == "joined"
            {
                return Ok(CommandResult::AlreadyJoined(
                    crate::protocol::results::AlreadyJoined {
                        thread: command.thread.clone(),
                        seat: command.seat.clone(),
                    },
                ));
            }
            let pending: Option<String> = tx.query_row(
                "SELECT i.id FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id) AND NOT EXISTS (SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.created_decision_seq=i.created_decision_seq) ORDER BY i.episode DESC LIMIT 1",
                params![command.thread.as_str(), command.seat.as_str()], |r| r.get(0),
            ).optional().map_err(store_error)?;
            if let Some(id) = pending {
                return Ok(CommandResult::Invitation(InvitationId::new(id)));
            }
            let deadline = schema::checked_deadline(decision.utc, duration)?;
            let episode = crate::store::service_controls::next_invitation_episode(
                tx,
                &command.thread,
                &command.seat,
            )?;
            if prior.is_some() {
                tx.execute("UPDATE memberships SET state='invited', voluntary_state='invited', episode=?1, joined_at=NULL, left_at=NULL WHERE thread_id=?2 AND seat_id=?3",
                    params![episode, command.thread.as_str(), command.seat.as_str()]).map_err(store_error)?;
            } else {
                tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,?2,?3,'invited','invited')",
                    params![command.thread.as_str(), command.seat.as_str(), episode]).map_err(store_error)?;
            }
            let invitation = InvitationId::new(crate::store::public_ids::fresh(
                tx,
                prefix::INVITATION,
                &[("invitations", "id", prefix::INVITATION)],
            )?);
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [command.seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let created_decision_seq = i64::try_from(schema::next_decision_seq(tx, &instance)?)
                .map_err(|_| {
                    api_error(
                        ErrorCode::SequenceExhausted,
                        "invitation decision sequence exhausted",
                    )
                })?;
            tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) VALUES (?1,?2,?3,?4,'pending',?5,?6,?7,?8,?9,?10)",
                params![invitation.as_str(), command.thread.as_str(), command.seat.as_str(), episode, decision.utc.0, created_decision_seq, duration, deadline.0,confirmation,caller.as_str()]).map_err(store_error)?;
            let mut payload = serde_json::json!({"action":"invite", "actor_seat":caller.as_str(), "seat":command.seat.as_str(),
                "invitation":invitation.as_str()});
            if timed {
                payload["deadline_at"] = serde_json::json!(deadline.0);
            }
            if let Some(mode) = confirmation {
                payload["confirmation"] = serde_json::json!(mode);
            }
            let payload = payload.to_string();
            schema::append_event_once(
                tx,
                EventInput {
                    thread: &command.thread,
                    key: &format!("invite:{}", invitation.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: Some(&invitation),
                },
            )?;
            schema::bump_membership_revision(tx, &command.thread)?;
            if timed {
                tx.execute("INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES (?1,1,1) ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|1,attention_version=attention_version+1", [command.seat.as_str()]).map_err(store_error)?;
            }
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &command.seat)?;
            schema::bump_filter_revision(tx, &instance, "inbox", command.seat.as_str())?;
            Ok(CommandResult::Invitation(invitation))
        },
    )
}

pub fn operator_orphan_invite(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    command: &OperatorOrphanInvite,
    actor: OperatorActor,
    installation_default_ms: Option<u64>,
) -> Result<CommandResult, ApiError> {
    let duration = command
        .deadline_millis
        .or(installation_default_ms)
        .unwrap_or(300_000);
    let duration = i64::try_from(duration)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invitation duration too large"))?;
    if duration <= 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invitation duration must be positive",
        ));
    }
    let scope = actor.operation_scope(instance);
    let typed_command = crate::protocol::commands::OperatorCommand::OrphanInvite(command.clone());
    let digest = crate::store::operator::digest(instance, &typed_command)?;
    let result = schema::execute_idempotent_transaction(
        context,
        conn,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            guard_pair_member(tx, &command.thread, &command.seat)?;
            let thread_instance: Option<String> = tx
                .query_row(
                    "SELECT instance_id FROM threads WHERE id=?1",
                    [command.thread.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            let target_instance: Option<String> = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1 AND state!='retired'",
                    [command.seat.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            if thread_instance.as_deref() != Some(instance)
                || target_instance.as_deref() != Some(instance)
            {
                return Err(api_error(
                    ErrorCode::TargetUnresolved,
                    "operator invite target outside thread instance",
                ));
            }
            let joined: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM memberships m JOIN seats s ON s.id=m.seat_id WHERE m.thread_id=?1 AND m.state='joined' AND s.state!='retired')", [command.thread.as_str()], |r| r.get(0)).map_err(store_error)?;
            if joined {
                return Err(api_error(
                    ErrorCode::ThreadNotOrphaned,
                    "thread still has joined seats",
                ));
            }
            Ok(())
        },
        |tx, decision| {
            let prior: Option<(i64, String)> = tx
                .query_row(
                    "SELECT episode,state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                    params![command.thread.as_str(), command.seat.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            let pending: Option<String> = tx.query_row("SELECT i.id FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id) AND NOT EXISTS (SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.created_decision_seq=i.created_decision_seq) ORDER BY i.episode DESC LIMIT 1",
                params![command.thread.as_str(),command.seat.as_str()], |r| r.get(0)).optional().map_err(store_error)?;
            if let Some(id) = pending {
                return Ok(CommandResult::OperatorInvited(InvitationId::new(id)));
            }
            let episode = crate::store::service_controls::next_invitation_episode(
                tx,
                &command.thread,
                &command.seat,
            )?;
            if prior.is_some() {
                tx.execute("UPDATE memberships SET state='invited',voluntary_state='invited',episode=?1,joined_at=NULL,left_at=NULL WHERE thread_id=?2 AND seat_id=?3",
                    params![episode,command.thread.as_str(),command.seat.as_str()]).map_err(store_error)?;
            } else {
                tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,?2,?3,'invited','invited')",
                    params![command.thread.as_str(),command.seat.as_str(),episode]).map_err(store_error)?;
            }
            let deadline = schema::checked_deadline(decision.utc, duration)?;
            let invitation = InvitationId::new(crate::store::public_ids::fresh(
                tx,
                prefix::INVITATION,
                &[("invitations", "id", prefix::INVITATION)],
            )?);
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [command.seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let created_decision_seq = i64::try_from(schema::next_decision_seq(tx, &instance)?)
                .map_err(|_| {
                    api_error(
                        ErrorCode::SequenceExhausted,
                        "invitation decision sequence exhausted",
                    )
                })?;
            tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES (?1,?2,?3,?4,'pending',?5,?6,?7,?8)",
                params![invitation.as_str(),command.thread.as_str(),command.seat.as_str(),episode,decision.utc.0,created_decision_seq,duration,deadline.0]).map_err(store_error)?;
            let payload = serde_json::json!({"action":"operator_orphan_invite", "actor":actor.audit_label(),
                "seat":command.seat.as_str(),"invitation":invitation.as_str(),"deadline_at":deadline.0}).to_string();
            schema::append_event_once(
                tx,
                EventInput {
                    thread: &command.thread,
                    key: &format!("operator_invite:{}", invitation.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: Some(&invitation),
                },
            )?;
            tx.execute("INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES (?1,1,1) ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|1,attention_version=attention_version+1",[command.seat.as_str()]).map_err(store_error)?;
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM threads WHERE id=?1",
                    [command.thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            schema::bump_membership_revision(tx, &command.thread)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &command.seat)?;
            schema::bump_filter_revision(tx, &instance, "inbox", command.seat.as_str())?;
            Ok(CommandResult::OperatorInvited(invitation))
        },
    )?;
    crate::store::operator::validate_result(&typed_command, &result)?;
    Ok(result)
}
