//! Invitations and consent: the commands `invite`, `operator_orphan_invite`,
//! `accept`, `reject` and `accept_required`, and the materializers of the
//! invitation fact kinds (`invitation.created`, `invitation.answered`).
//!
//! Responsibility
//! - Owns: the commands (decide under A2, then author the fact through
//!   [`FactRoute`]), and the one writer of `invitations` and
//!   `invitation_rejections` for a new invitation or an answer
//!   ([`materialize_invitation`], [`materialize_answer`]); a service writer
//!   reaches them through the node-local route.
//! - Must not: write an invitation row outside a materializer, nor reject a
//!   fact because of the local membership view (receivers apply; only the
//!   origin-authority check rejects). The `already_joined` rule is ordinals
//!   only (`last_joined_n > invitee_n`), never the current membership state.
//! - Role: `sync::facts::invitation` handlers and the local-only route call the
//!   materializers.

use super::membership::{JoinedTransition, materialize_joined, membership_route, next_transition};
use super::thread::{has_creating_fact, local_instance};
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
use crate::store::attribution;
use crate::store::connection::{DecisionInstant, StoreContext, api_error, store_error};
use crate::store::delivery_tail::DeliveryTail;
use crate::store::materialize::{FactRoute, MaterializeInput, local_only};
use crate::store::schema::{self, EventInput};
use crate::sync::facts::invitation::{
    InvitationAnswered, InvitationAnsweredPayload, InvitationCreated, InvitationCreatedPayload,
};
use crate::sync::facts::payload::{AnswerOutcome, Confirmation, InviterKind, answer_key};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

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
                context,
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
            // The acceptance is the invitee's one fact: its membership
            // ordinal is the next one, and the attribution of the local
            // binding rides in the staging table until the materializer takes it.
            let n = next_transition(tx, &thread, &caller)?;
            attribution::stage(
                tx,
                &attribution::answer_key(invitation.as_str()),
                &actor,
                &actor.observation(decision.utc.0),
            )?;
            author_answer(
                tx,
                context,
                decision,
                &InvitationAnsweredPayload {
                    invitation: invitation.clone(),
                    invitee: caller.clone(),
                    outcome: AnswerOutcome::Accepted,
                    reason: None,
                    answered_at: decision.utc.0,
                    n,
                },
            )?;
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
                context,
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
            // Lateness is retained, never described as expiration or
            // acceptance: the materializer records it, on the deciding node.
            attribution::stage(
                tx,
                &attribution::answer_key(command.invitation.as_str()),
                &actor,
                &actor.observation(decision.utc.0),
            )?;
            author_answer(
                tx,
                context,
                decision,
                &InvitationAnsweredPayload {
                    invitation: command.invitation.clone(),
                    invitee: caller.clone(),
                    outcome: AnswerOutcome::Rejected,
                    reason: Some(command.reason.clone()),
                    answered_at: decision.utc.0,
                    n: 0,
                },
            )?;
            let (seat, generation, observation, at, reason): (String, i64, String, i64, String) = tx
                .query_row(
                    "SELECT actor_seat_id,generation,observation,rejected_at,reason FROM invitation_rejections WHERE invitation_id=?1",
                    [command.invitation.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()
                .map_err(store_error)?
                .ok_or_else(|| api_error(ErrorCode::Conflict, "invitation was not rejected"))?;
            Ok(CommandResult::Rejected(
                crate::protocol::results::InvitationRejection {
                    invitation: command.invitation.clone(),
                    actor: SeatId::new(seat),
                    generation: generation as u64,
                    observation,
                    rejected_at: UtcMillis(at),
                    reason,
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
                context,
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            // A requirement lives on a managed thread, which is node-local:
            // the writes below go through the local-only route (its input
            // holds the replicated-write scope for the rest of this closure).
            let local = local_only(tx, decision)?;
            let generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let observation = actor.observation(decision.utc.0);
            schema::record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, decision.utc)?;
            mark_accepted(
                tx,
                &command.invitation,
                &caller,
                decision.utc.0,
                generation,
                &observation,
            )?;
            schema::clear_warning_condition_for_invitation(
                tx,
                command.invitation.as_str(),
                decision.utc,
            )?;
            tx.execute("UPDATE requirement_episodes SET state='accepted',revision=revision+1,accepted_at=?1,accepted_by_seat_id=?2,accepted_generation=?3,accepted_observation=?4 WHERE id=?5 AND state='pending' AND revision=?6",
                params![decision.utc.0,caller.as_str(),generation,observation,command.requirement.as_str(),command.expected_revision as i64]).map_err(store_error)?;
            let instance = local_instance(tx)?;
            let joined = joined_in_thread(tx, &command.thread, &caller)?;
            let episode: i64 = tx
                .query_row(
                    "SELECT episode FROM invitations WHERE id=?1",
                    [command.invitation.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            // The transition takes a position after the late-overdue warning
            // above, as the acceptance always did.
            let seq = if joined {
                local.decision_seq
            } else {
                schema::next_decision_seq(tx, &instance)?
            };
            materialize_joined(
                tx,
                &local,
                &JoinedTransition {
                    thread: &command.thread,
                    seat: &caller,
                    n: next_transition(tx, &command.thread, &caller)?,
                    at: decision.utc.0,
                    join_event: false,
                    seq,
                    episode: Some(episode),
                    direct_floor: None,
                    // The requirement's own invitation episodes stay independent.
                    settle: false,
                },
            )?;
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
        crate::protocol::commands::InvitationConfirmation::Lazy => Confirmation::Lazy,
        crate::protocol::commands::InvitationConfirmation::RequireAck => Confirmation::RequireAck,
    });
    let timed = confirmation != Some(Confirmation::Lazy);
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
                context,
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
            let invitation = InvitationId::new(crate::store::public_ids::fresh(
                tx,
                prefix::INVITATION,
                &[("invitations", "id", prefix::INVITATION)],
            )?);
            let invitee_n = home_transition(tx, &command.thread, &command.seat)?;
            author_invitation(
                tx,
                context,
                decision,
                &InvitationCreatedPayload {
                    invitation: invitation.clone(),
                    thread: command.thread.clone(),
                    inviter: Some(caller.clone()),
                    inviter_kind: InviterKind::Seat,
                    invitee: command.seat.clone(),
                    invitee_n,
                    created_at: decision.utc.0,
                    duration_ms: duration,
                    deadline_at: timed.then_some(deadline.0),
                    confirmation,
                    actor_label: None,
                },
            )?;
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
            let pending: Option<String> = tx.query_row("SELECT i.id FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id) AND NOT EXISTS (SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.created_decision_seq=i.created_decision_seq) ORDER BY i.episode DESC LIMIT 1",
                params![command.thread.as_str(),command.seat.as_str()], |r| r.get(0)).optional().map_err(store_error)?;
            if let Some(id) = pending {
                return Ok(CommandResult::OperatorInvited(InvitationId::new(id)));
            }
            let deadline = schema::checked_deadline(decision.utc, duration)?;
            let invitation = InvitationId::new(crate::store::public_ids::fresh(
                tx,
                prefix::INVITATION,
                &[("invitations", "id", prefix::INVITATION)],
            )?);
            let invitee_n = home_transition(tx, &command.thread, &command.seat)?;
            author_invitation(
                tx,
                context,
                decision,
                &InvitationCreatedPayload {
                    invitation: invitation.clone(),
                    thread: command.thread.clone(),
                    inviter: None,
                    inviter_kind: InviterKind::Operator,
                    invitee: command.seat.clone(),
                    invitee_n,
                    created_at: decision.utc.0,
                    duration_ms: duration,
                    deadline_at: Some(deadline.0),
                    confirmation: None,
                    actor_label: Some(actor.audit_label()),
                },
            )?;
            Ok(CommandResult::OperatorInvited(invitation))
        },
    )?;
    crate::store::operator::validate_result(&typed_command, &result)?;
    Ok(result)
}

/// The invitee's `memberships.home_transition` for the thread in this node's
/// view (0 without a row): the `invitee_n` an invitation records.
fn home_transition(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<u64, ApiError> {
    let n: Option<i64> = tx
        .query_row(
            "SELECT home_transition FROM memberships WHERE thread_id=?1 AND seat_id=?2",
            params![thread.as_str(), seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    u64::try_from(n.unwrap_or(0))
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative membership ordinal"))
}

/// Authors `invitation.created`: through the journal for a journaled thread,
/// as a node-local write for any other (a managed thread, which refuses a
/// remote seat).
pub(crate) fn author_invitation(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    payload: &InvitationCreatedPayload,
) -> Result<(), ApiError> {
    membership_route(tx, &payload.thread)?.emit::<InvitationCreated>(
        tx,
        ctx,
        decision,
        payload,
        None,
        materialize_invitation,
    )?;
    Ok(())
}

/// Authors `invitation.answered` the way [`author_invitation`] authors a new
/// invitation.
fn author_answer(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    payload: &InvitationAnsweredPayload,
) -> Result<(), ApiError> {
    let thread: String = tx
        .query_row(
            "SELECT thread_id FROM invitations WHERE id=?1",
            [payload.invitation.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let thread = ThreadId::new(thread);
    // An invitation no fact created (a fixture row) answers node-locally too:
    // its `invitation.answered` would park on the invitation key.
    let route = match membership_route(tx, &thread)? {
        FactRoute::Journal if !has_creating_fact(tx, payload.invitation.as_str())? => {
            FactRoute::LocalOnly
        }
        route => route,
    };
    route.emit::<InvitationAnswered>(tx, ctx, decision, payload, None, materialize_answer)?;
    Ok(())
}

/// The seat an invitation is addressed to, if the invitation exists.
pub fn invitation_invitee(
    tx: &Transaction<'_>,
    invitation: &InvitationId,
) -> Result<Option<SeatId>, ApiError> {
    tx.query_row(
        "SELECT seat_id FROM invitations WHERE id=?1",
        [invitation.as_str()],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map(|seat| seat.map(SeatId::new))
    .map_err(store_error)
}

/// Where a new invitation leaves the invitee's membership row. A voluntary
/// invitation (every replicated one) makes the invitee `invited`; a service
/// requirement keeps the voluntary state the seat already had (a node-local
/// writer only, on a managed thread).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    Voluntary,
    Required,
}

/// Materializes `invitation.created` (fact-kinds design section 6): a no-op if
/// the invitation exists. Otherwise inserts the row with a local episode and
/// `origin_node = m.origin_node`, in the state the invitee's seat and ordinals
/// give it (`recipient_retired` for a retired seat, `already_joined` when
/// `last_joined_n > invitee_n`, else `pending`). Only a `pending` row touches
/// anything else: the invitee becomes `invited`, the derived invite event is
/// appended, the revisions and filters bump, and a local, non-suppressed
/// invitee gets its delivery attention.
pub fn materialize_invitation(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &InvitationCreatedPayload,
) -> Result<(), ApiError> {
    insert_invitation(tx, m, payload, Placement::Voluntary, true).map(|_| ())
}

/// [`materialize_invitation`] with the membership placement and the derived
/// event chosen by the caller. Returns the new row's episode, `None` when the
/// invitation existed.
pub(crate) fn insert_invitation(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    p: &InvitationCreatedPayload,
    placement: Placement,
    event: bool,
) -> Result<Option<i64>, ApiError> {
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM invitations WHERE id=?1)",
            [p.invitation.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if exists {
        return Ok(None);
    }
    let (seat_state, seat_retired_at): (String, Option<i64>) = tx
        .query_row(
            "SELECT state,retired_at FROM seats WHERE id=?1",
            [p.invitee.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| {
            api_error(
                ErrorCode::NotFound,
                format!("unknown invitee seat {}", p.invitee.as_str()),
            )
        })?;
    let prior: Option<(String, i64)> = tx
        .query_row(
            "SELECT state,last_joined_n FROM memberships WHERE thread_id=?1 AND seat_id=?2",
            params![p.thread.as_str(), p.invitee.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let invitee_n = i64::try_from(p.invitee_n)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invitee ordinal out of range"))?;
    let state = if seat_state == "retired" {
        "recipient_retired"
    } else if prior
        .as_ref()
        .is_some_and(|(_, joined_n)| *joined_n > invitee_n)
    {
        "already_joined"
    } else {
        "pending"
    };
    let deadline = match p.deadline_at {
        Some(deadline) => deadline,
        None => p.created_at.checked_add(p.duration_ms).ok_or_else(|| {
            api_error(
                ErrorCode::InvalidRequest,
                "invitation deadline out of range",
            )
        })?,
    };
    let episode =
        crate::store::service_controls::next_invitation_episode(tx, &p.thread, &p.invitee)?;
    let inviter = match p.inviter_kind {
        InviterKind::Seat => p.inviter.as_ref().map(SeatId::as_str),
        _ => None,
    };
    let decision_seq = i64::try_from(m.decision_seq)
        .map_err(|_| api_error(ErrorCode::SequenceExhausted, "decision sequence exhausted"))?;
    tx.execute(
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,retired_at,confirmation,inviter_seat_id,origin_node,invitee_n,inviter_kind) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            p.invitation.as_str(),
            p.thread.as_str(),
            p.invitee.as_str(),
            episode,
            state,
            p.created_at,
            decision_seq,
            p.duration_ms,
            deadline,
            if state == "recipient_retired" { seat_retired_at } else { None },
            p.confirmation.map(Confirmation::as_str),
            inviter,
            m.origin_node.to_string(),
            invitee_n,
            p.inviter_kind.as_str(),
        ],
    )
    .map_err(store_error)?;
    if state == "recipient_retired" && prior.is_none() {
        // The terminal row the retirement job leaves for an invitee whose
        // invitation applied first: either arrival order ends with the same
        // `memberships` row (fact-kinds design section 3 audit; the order
        // permutation suite).
        tx.execute(
            "INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state,retired_at) VALUES (?1,?2,?3,'retired','retired',?4)",
            params![p.thread.as_str(), p.invitee.as_str(), episode, seat_retired_at],
        )
        .map_err(store_error)?;
        schema::bump_membership_revision(tx, &p.thread)?;
    }
    if state != "pending" {
        return Ok(Some(episode));
    }
    place_invitee(
        tx,
        p,
        placement,
        prior.as_ref().map(|(state, _)| state.as_str()),
        episode,
    )?;
    let timed = p.confirmation != Some(Confirmation::Lazy);
    if event {
        invite_event(tx, m, p, deadline, episode)?;
    }
    let instance = local_instance(tx)?;
    schema::bump_membership_revision(tx, &p.thread)?;
    if crate::store::seats::is_local_seat(tx, &p.invitee)?
        && !(m.suppress)(
            tx,
            &answer_key(&p.invitation)
                .map_err(|e| api_error(ErrorCode::InvalidRequest, e.to_string()))?,
        )?
    {
        DeliveryTail::invitation(tx, &p.invitee, timed)?;
    }
    schema::bump_filter_revision(tx, &instance, "directory", "all")?;
    bump_member_directory(tx, &instance, &p.invitee)?;
    schema::bump_filter_revision(tx, &instance, "inbox", p.invitee.as_str())?;
    Ok(Some(episode))
}

/// Makes the invitee `invited` (a voluntary invitation) or leaves the
/// voluntary state alone (a requirement): the membership half of a new
/// pending invitation.
fn place_invitee(
    tx: &Transaction<'_>,
    p: &InvitationCreatedPayload,
    placement: Placement,
    prior_state: Option<&str>,
    episode: i64,
) -> Result<(), ApiError> {
    let (thread, seat) = (p.thread.as_str(), p.invitee.as_str());
    match (placement, prior_state) {
        (Placement::Voluntary, None) => {
            tx.execute(
                "INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,?2,?3,'invited','invited')",
                params![thread, seat, episode],
            )
            .map_err(store_error)?;
        }
        // A joined seat stays joined (a forked log can deliver an invitation
        // the seat's own transitions already answered); a retired row is terminal.
        (Placement::Voluntary, Some("joined" | "retired")) => {}
        (Placement::Voluntary, Some(_)) => {
            tx.execute(
                "UPDATE memberships SET state='invited', voluntary_state='invited', episode=?1, joined_at=NULL, left_at=NULL WHERE thread_id=?2 AND seat_id=?3",
                params![episode, thread, seat],
            )
            .map_err(store_error)?;
        }
        (Placement::Required, None) => {
            tx.execute(
                "INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,?2,?3,'invited','absent')",
                params![thread, seat, episode],
            )
            .map_err(store_error)?;
        }
        (Placement::Required, Some("left")) => {
            tx.execute(
                "UPDATE memberships SET state='invited',voluntary_state='left' WHERE thread_id=?1 AND seat_id=?2",
                params![thread, seat],
            )
            .map_err(store_error)?;
        }
        (Placement::Required, Some(_)) => {}
    }
    Ok(())
}

/// The derived timeline event of a new pending invitation, in the shape its
/// issuer always had: `invite:` for a seat, `operator_invite:` for an
/// operator, `service_invite:<id>:ordinary` for a service.
fn invite_event(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    p: &InvitationCreatedPayload,
    deadline: i64,
    episode: i64,
) -> Result<(), ApiError> {
    let id = p.invitation.as_str();
    let (key, payload, author) = match p.inviter_kind {
        InviterKind::Seat => {
            let mut payload = serde_json::json!({"action":"invite",
                "actor_seat":p.inviter.as_ref().map(SeatId::as_str),
                "seat":p.invitee.as_str(),"invitation":id});
            if p.confirmation != Some(Confirmation::Lazy) {
                payload["deadline_at"] = serde_json::json!(deadline);
            }
            if let Some(mode) = p.confirmation {
                payload["confirmation"] = serde_json::json!(mode.as_str());
            }
            (format!("invite:{id}"), payload, None)
        }
        InviterKind::Operator => (
            format!("operator_invite:{id}"),
            serde_json::json!({"action":"operator_orphan_invite",
                "actor":p.actor_label.as_deref().unwrap_or("operator"),
                "seat":p.invitee.as_str(),"invitation":id,"deadline_at":deadline}),
            None,
        ),
        InviterKind::Service => {
            let issuer = p.actor_label.as_deref().unwrap_or("service");
            // A service author is a node-local registration: only its own
            // node can attribute the event to it.
            let registered: bool = m.local_origin
                && tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM service_authors WHERE id=?1)",
                        [issuer],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
            (
                format!("service_invite:{id}:ordinary"),
                serde_json::json!({"action":"service_invite","issuer":issuer,
                    "seat":p.invitee.as_str(),"invitation":id,"required":false,
                    "episode":episode}),
                registered.then(|| {
                    crate::protocol::service::EventAuthor::Programmatic(
                        crate::protocol::ids::ServiceAuthorId::new(issuer),
                    )
                }),
            )
        }
    };
    let payload = payload.to_string();
    let input = EventInput {
        thread: &p.thread,
        key: &key,
        kind: "info",
        payload_json: &payload,
        decision_at: UtcMillis(p.created_at),
        source_message: None,
        source_invitation: Some(&p.invitation),
    };
    match author {
        Some(author) => schema::append_attributed_event_once(tx, input, author)?,
        None => schema::append_event_once(tx, input)?,
    };
    Ok(())
}

/// Marks a pending invitation `accepted` with the acceptor's attribution.
pub(crate) fn mark_accepted(
    tx: &Transaction<'_>,
    invitation: &InvitationId,
    seat: &SeatId,
    at: i64,
    generation: i64,
    observation: &str,
) -> Result<usize, ApiError> {
    tx.execute(
        "UPDATE invitations SET state='accepted', accepted_at=?1,accepted_actor_seat_id=?2,accepted_generation=?3,accepted_observation=?4 WHERE id=?5 AND state='pending'",
        params![at, seat.as_str(), generation, observation, invitation.as_str()],
    )
    .map_err(store_error)
}

/// Where the next event of an answer lands: the entry's own `decision_seq`
/// while nothing allocated a later one (the common case), else a fresh one.
fn event_position(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    instance: &str,
) -> Result<u64, ApiError> {
    let current: i64 = tx
        .query_row(
            "SELECT decision_seq FROM local_node WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if u64::try_from(current).is_ok_and(|current| current == m.decision_seq) {
        Ok(m.decision_seq)
    } else {
        schema::next_decision_seq(tx, instance)
    }
}

/// Materializes `invitation.answered` (section 6): a no-op unless the
/// invitation is `pending` (not rejected, not cancelled). A late answer records
/// the overdue warning first (`record_overdue_if_pending` acts only on the
/// deciding node). An accept marks the invitation accepted from the staged
/// `answer:<id>` attribution, applies the invitee's membership joined
/// transition with the payload's ordinal (which settles the invitee's other
/// pending invitations `already_joined`) and appends the `accept:` event. A
/// reject records the rejection, restores the invitee's voluntary state and
/// appends the rejection warning. Both clear the invitation's warning condition.
pub fn materialize_answer(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &InvitationAnsweredPayload,
) -> Result<(), ApiError> {
    // Whatever the outcome, the staged attribution of this answer is consumed.
    let attribution =
        attribution::take(tx, m, &attribution::answer_key(payload.invitation.as_str()))?;
    let row: Option<(String, String, i64, bool)> = tx
        .query_row(
            "SELECT i.thread_id,i.seat_id,i.episode,\
             i.state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=i.id) \
             AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) \
             FROM invitations i WHERE i.id=?1",
            [payload.invitation.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((thread, seat, episode, open)) = row else {
        return Err(api_error(ErrorCode::NotFound, "invitation not found"));
    };
    if !open {
        return Ok(());
    }
    let (thread, seat) = (ThreadId::new(thread), SeatId::new(seat));
    let journaled: bool = tx
        .query_row(
            "SELECT journaled FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let instance = local_instance(tx)?;
    let at = UtcMillis(payload.answered_at);
    let obligation = ObligationRef::Invitation(payload.invitation.clone());
    match payload.outcome {
        AnswerOutcome::Accepted => {
            // The pair episode's read floor, captured before the state changes.
            // Only a live local decision has it: it staged the acceptor's
            // binding, which a replicated or re-ingested answer never has.
            let direct_floor = if attribution.binding_id.is_some()
                && !joined_in_thread(tx, &thread, &seat)?
                && crate::store::seats::is_local_seat(tx, &seat)?
            {
                crate::store::lazy_delivery::direct_acceptance_floor(
                    tx,
                    &instance,
                    &seat,
                    &thread,
                    &payload.invitation,
                )?
            } else {
                None
            };
            schema::record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, at)?;
            mark_accepted(
                tx,
                &payload.invitation,
                &seat,
                payload.answered_at,
                attribution.generation,
                &attribution.observation,
            )?;
            schema::clear_warning_condition_for_invitation(tx, payload.invitation.as_str(), at)?;
            let seq = event_position(tx, m, &instance)?;
            materialize_joined(
                tx,
                m,
                &JoinedTransition {
                    thread: &thread,
                    seat: &seat,
                    n: payload.n,
                    at: payload.answered_at,
                    join_event: false,
                    seq,
                    episode: Some(episode),
                    direct_floor,
                    settle: journaled,
                },
            )?;
            let event = serde_json::json!({"action":"accept","seat":seat.as_str(),
                "invitation":payload.invitation.as_str()})
            .to_string();
            schema::append_event_once_with_decision_seq(
                tx,
                EventInput {
                    thread: &thread,
                    key: &format!("accept:{}", payload.invitation.as_str()),
                    kind: "info",
                    payload_json: &event,
                    decision_at: at,
                    source_message: None,
                    source_invitation: Some(&payload.invitation),
                },
                seq,
            )?;
        }
        AnswerOutcome::Rejected => {
            let reason = payload.reason.as_deref().unwrap_or_default();
            schema::record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, at)?;
            tx.execute(
                "INSERT INTO invitation_rejections(invitation_id,reason,rejected_at,actor_seat_id,generation,observation) VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    payload.invitation.as_str(),
                    reason,
                    payload.answered_at,
                    seat.as_str(),
                    attribution.generation,
                    attribution.observation
                ],
            )
            .map_err(store_error)?;
            schema::clear_warning_condition_for_invitation(tx, payload.invitation.as_str(), at)?;
            // Only an invited voluntary marker is cleared; no join or leave interval is fabricated.
            // Restore a prior left state only with its recorded closed interval and leave event.
            let prior_left: Option<i64> = tx.query_row(
                "SELECT m.decision_at FROM membership_intervals mi JOIN messages m ON m.thread_id=mi.thread_id AND m.decision_seq=mi.left_seq WHERE mi.thread_id=?1 AND mi.seat_id=?2 AND mi.left_seq IS NOT NULL AND json_extract(m.event_json,'$.action')='leave' AND json_extract(m.event_json,'$.seat')=?2 ORDER BY mi.episode DESC LIMIT 1",
                params![thread.as_str(), seat.as_str()], |r| r.get(0),
            ).optional().map_err(store_error)?;
            tx.execute("UPDATE memberships SET voluntary_state=?1,left_at=?2 WHERE thread_id=?3 AND seat_id=?4 AND voluntary_state='invited' AND episode=(SELECT episode FROM invitations WHERE id=?5) AND NOT EXISTS(SELECT 1 FROM invitations i WHERE i.thread_id=?3 AND i.seat_id=?4 AND i.state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=i.id))",
                params![if prior_left.is_some() { "left" } else { "absent" }, prior_left, thread.as_str(), seat.as_str(), payload.invitation.as_str()]).map_err(store_error)?;
            let event = serde_json::json!({"action":"reject","seat":seat.as_str(),
                "invitation":payload.invitation.as_str(),"reason":reason})
            .to_string();
            let (warning, inserted) = schema::append_attributed_event_once(
                tx,
                EventInput {
                    thread: &thread,
                    key: &format!("reject:{}", payload.invitation.as_str()),
                    kind: "warn",
                    payload_json: &event,
                    decision_at: at,
                    source_message: None,
                    source_invitation: Some(&payload.invitation),
                },
                crate::protocol::service::EventAuthor::Native(seat.clone()),
            )?;
            if inserted {
                schema::enqueue_invitation_rejection_notice(
                    tx,
                    warning.as_str(),
                    thread.as_str(),
                    seat.as_str(),
                )?;
            }
        }
    }
    schema::bump_membership_revision(tx, &thread)?;
    schema::bump_filter_revision(tx, &instance, "directory", "all")?;
    bump_member_directory(tx, &instance, &seat)?;
    schema::bump_filter_revision(tx, &instance, "inbox", seat.as_str())?;
    Ok(())
}
