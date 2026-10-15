//! Direct-pair channels.

use super::{bump_member_directory, cooperative_payload_hash, decide_accountable, validate_actor};
use crate::protocol::authority::{MutationPermit, ObligationRef};
use crate::protocol::ids::{InvitationId, SeatId, ThreadId, prefix};
use crate::protocol::results::{ApiError, CommandResult, ErrorCode};
use crate::protocol::time::{CallBudget, UtcMillis};
use crate::store::connection::{DecisionInstant, StoreContext, api_error, store_error};
use crate::store::schema::{self, EventInput};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// The registry alone identifies an automatic channel. Reject inconsistent
/// audience rows rather than letting a raw ordinary send broadcast them.
pub(crate) fn direct_pair(
    db: &Connection,
    thread: &ThreadId,
) -> Result<Option<(SeatId, SeatId)>, ApiError> {
    let row: Option<(String,String,bool)> = db.query_row(
        "SELECT d.seat_a,d.seat_b,d.instance_id=t.instance_id AND a.instance_id=d.instance_id AND b.instance_id=d.instance_id AND t.managed_owner_author_id IS NULL FROM direct_threads d JOIN threads t ON t.id=d.thread_id JOIN seats a ON a.id=d.seat_a JOIN seats b ON b.id=d.seat_b WHERE d.thread_id=?1",
        [thread.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?;
    let Some((a, b, valid)) = row else {
        return Ok(None);
    };
    if !valid {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "automatic pair has inconsistent ownership or participants",
        ));
    }
    // Three indexed ranges avoid walking arbitrarily many historical intervals
    // for the two legitimate seats just to prove there is no third seat.
    for table in [
        "memberships",
        "membership_intervals",
        "requirement_episodes",
    ] {
        let sql = format!(
            "SELECT EXISTS(SELECT 1 FROM {table} WHERE thread_id=?1 AND seat_id<?2) OR EXISTS(SELECT 1 FROM {table} WHERE thread_id=?1 AND seat_id>?2 AND seat_id<?3) OR EXISTS(SELECT 1 FROM {table} WHERE thread_id=?1 AND seat_id>?3)"
        );
        let extra: bool = db
            .query_row(&sql, params![thread.as_str(), a, b], |r| r.get(0))
            .map_err(store_error)?;
        if extra {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "automatic pair has inconsistent ownership or participants",
            ));
        }
    }
    Ok(Some((SeatId::new(a), SeatId::new(b))))
}

pub(crate) fn guard_pair_member(
    db: &Connection,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<(), ApiError> {
    if let Some((a, b)) = direct_pair(db, thread)?
        && seat != &a
        && seat != &b
    {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "automatic channel reserves exactly its original two seats",
        ));
    }
    Ok(())
}

pub(crate) fn guard_pair_service_owner(db: &Connection, thread: &ThreadId) -> Result<(), ApiError> {
    if direct_pair(db, thread)?.is_some() {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "services cannot own automatic pair channels",
        ));
    }
    Ok(())
}

fn require_pair_lifecycle(
    db: &Connection,
    thread: &ThreadId,
    caller: &SeatId,
    recipient: &SeatId,
) -> Result<(), ApiError> {
    let archived: bool = db
        .query_row(
            "SELECT archived FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if archived {
        return Err(api_error(
            ErrorCode::Archived,
            format!(
                "automatic channel {thread} is archived; reopen {thread} explicitly",
                thread = thread.as_str()
            ),
        ));
    }
    let sender = crate::store::service_substrate::effective_membership(db, thread, caller)?;
    if sender.native_state != Some("joined") {
        let action = if sender.native_state == Some("invited") {
            "accept"
        } else {
            "join"
        };
        return Err(api_error(
            ErrorCode::Conflict,
            format!(
                "caller must explicitly {action} {thread} before direct send",
                thread = thread.as_str()
            ),
        ));
    }
    let peer = crate::store::service_substrate::effective_membership(db, thread, recipient)?;
    if !matches!(peer.native_state, Some("joined" | "invited")) {
        return Err(api_error(
            ErrorCode::Conflict,
            format!(
                "target must be explicitly reinvited: invite {thread} --seat {recipient}",
                thread = thread.as_str(),
                recipient = recipient.as_str()
            ),
        ));
    }
    Ok(())
}

/// Validate both marked and raw ordinary sends against the immutable universe.
pub(crate) fn validate_pair_send(
    db: &Connection,
    request: &crate::protocol::commands::SendMessage,
) -> Result<(), ApiError> {
    let Some((a, b)) = direct_pair(db, &request.thread)? else {
        return if request.direct_recipient.is_some() {
            Err(api_error(
                ErrorCode::InvalidRequest,
                "direct recipient requires its canonical automatic channel",
            ))
        } else {
            Ok(())
        };
    };
    let caller = &request.claim.seat;
    if caller != &a && caller != &b {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "sender outside automatic pair",
        ));
    }
    if request
        .invited_recipients
        .iter()
        .any(|s| s != &a && s != &b)
    {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "automatic channel reserves exactly its original two seats",
        ));
    }
    if let Some(recipient) = &request.direct_recipient {
        let other = if caller == &a { &b } else { &a };
        if recipient != other || request.invited_recipients.iter().any(|s| s != other) {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "direct recipient and ACK audience must name only the other pair seat",
            ));
        }
        crate::store::seats::eligible_delivery_recipient(db, &request.claim.instance, recipient)?;
        require_pair_lifecycle(db, &request.thread, caller, recipient)?;
    }
    Ok(())
}

pub fn ensure_direct_thread(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &crate::protocol::commands::EnsureDirectThread,
    mut permit: MutationPermit,
    installation_default_ms: Option<u64>,
) -> Result<CommandResult, ApiError> {
    if let Some(policy) = &command.creation_policy {
        policy
            .validate()
            .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    }
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("ensure_direct_thread", command)?;
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| validate_actor(tx, &caller, command.claim.target.as_str()).map(|_| ()),
        |tx, decision| {
            decide_accountable(
                context,
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &ObligationRef::CheckIn(caller.clone()),
                &digest,
            )?;
            let (thread, created) = resolve_direct_pair(
                tx,
                decision,
                &command.claim,
                &command.recipient,
                command.creation_policy.as_ref(),
                installation_default_ms,
            )?;
            direct_pair_selection(
                tx,
                thread,
                &command.recipient,
                created,
                command.creation_policy.is_some(),
            )
        },
    )
}

/// One resolver for ordinary Ensure and future attached handoff children. The
/// caller owns the accountable transaction and must decide A2 before calling.
pub(crate) fn resolve_direct_pair(
    tx: &Transaction<'_>,
    decision: DecisionInstant,
    claim: &crate::protocol::authority::CallerClaim,
    recipient: &SeatId,
    creation_policy: Option<&crate::protocol::commands::EnsureCreationPolicy>,
    installation_default_ms: Option<u64>,
) -> Result<(ThreadId, bool), ApiError> {
    let instance = claim.instance.as_str();
    let caller = &claim.seat;
    if caller == recipient {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "direct recipient must differ from caller",
        ));
    }
    crate::store::seats::eligible_delivery_recipient(tx, instance, recipient)?;
    let (a, b) = if caller < recipient {
        (caller, recipient)
    } else {
        (recipient, caller)
    };
    let existing: Option<String> = tx
        .query_row(
            "SELECT thread_id FROM direct_threads WHERE instance_id=?1 AND seat_a=?2 AND seat_b=?3",
            params![instance, a.as_str(), b.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if let Some(id) = existing {
        let thread = ThreadId::new(id);
        direct_pair(tx, &thread)?;
        require_pair_lifecycle(tx, &thread, caller, recipient)?;
        return Ok((thread, false));
    }
    if let Some(policy) = creation_policy {
        policy
            .validate()
            .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    }
    if creation_policy.is_some_and(|p| {
        p.confirmation == crate::protocol::commands::InvitationConfirmation::RequireAck
    }) && claim.harness == crate::protocol::authority::Harness::Human
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "human invitation cannot require agent confirmation",
        ));
    }
    let confirmation = creation_policy.map(|p| match p.confirmation {
        crate::protocol::commands::InvitationConfirmation::Lazy => "lazy",
        crate::protocol::commands::InvitationConfirmation::RequireAck => "require_ack",
    });
    let timed = confirmation != Some("lazy");
    let duration = i64::try_from(
        creation_policy
            .and_then(|p| p.deadline_millis)
            .or(installation_default_ms)
            .unwrap_or(300_000),
    )
    .map_err(|_| api_error(ErrorCode::InvalidRequest, "invitation duration too large"))?;
    if duration <= 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invitation duration must be positive",
        ));
    }
    let deadline = schema::checked_deadline(decision.utc, duration)?;
    let thread = ThreadId::new(crate::store::public_ids::fresh(
        tx,
        prefix::THREAD,
        &[("threads", "id", prefix::THREAD)],
    )?);
    let label = format!("1:1 {} {}", a.as_str(), b.as_str());
    let joined_seq = schema::next_decision_seq(tx, instance)?;
    tx.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES(?1,?2,?3,?3,?4,?4)",params![thread.as_str(),instance,label,decision.utc.0]).map_err(store_error)?;
    tx.execute("INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) VALUES(?1,?2,'joined','joined',?3)",params![thread.as_str(),caller.as_str(),decision.utc.0]).map_err(store_error)?;
    crate::store::read_cursors::init_for_join(tx, caller, &thread)?;
    tx.execute(
        "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES(?1,?2,1,?3)",
        params![thread.as_str(), caller.as_str(), joined_seq as i64],
    )
    .map_err(store_error)?;
    let payload =
        serde_json::json!({"action":"create_thread","goal":label,"actor_seat":caller.as_str()})
            .to_string();
    schema::append_event_once_with_decision_seq(
        tx,
        EventInput {
            thread: &thread,
            key: &format!("create:{}", thread.as_str()),
            kind: "info",
            payload_json: &payload,
            decision_at: decision.utc,
            source_message: None,
            source_invitation: None,
        },
        joined_seq,
    )?;
    let invitation = InvitationId::new(crate::store::public_ids::fresh(
        tx,
        prefix::INVITATION,
        &[("invitations", "id", prefix::INVITATION)],
    )?);
    let invited_seq = schema::next_decision_seq(tx, instance)?;
    tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES(?1,?2,1,'invited','invited')",params![thread.as_str(),recipient.as_str()]).map_err(store_error)?;
    tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) VALUES(?1,?2,?3,1,'pending',?4,?5,?6,?7,?8,?9)",params![invitation.as_str(),thread.as_str(),recipient.as_str(),decision.utc.0,invited_seq as i64,duration,deadline.0,confirmation,creation_policy.map(|_| caller.as_str())]).map_err(store_error)?;
    let mut payload = serde_json::json!({"action":"invite","actor_seat":caller.as_str(),"seat":recipient.as_str(),"invitation":invitation.as_str()});
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
            thread: &thread,
            key: &format!("invite:{}", invitation.as_str()),
            kind: "info",
            payload_json: &payload,
            decision_at: decision.utc,
            source_message: None,
            source_invitation: Some(&invitation),
        },
    )?;
    tx.execute(
        "INSERT INTO direct_threads(instance_id,seat_a,seat_b,thread_id) VALUES(?1,?2,?3,?4)",
        params![instance, a.as_str(), b.as_str(), thread.as_str()],
    )
    .map_err(store_error)?;
    for seat in [caller, recipient] {
        // Preserve each ordinary membership transition's revision/projection bump.
        schema::bump_membership_revision(tx, &thread)?;
        schema::bump_filter_revision(tx, instance, "directory", "all")?;
        bump_member_directory(tx, instance, seat)?;
        schema::bump_filter_revision(tx, instance, "inbox", seat.as_str())?;
    }
    if timed {
        tx.execute("INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES(?1,1,1) ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|1,attention_version=attention_version+1",[recipient.as_str()]).map_err(store_error)?;
    }
    Ok((thread, true))
}

pub(crate) fn direct_pair_selection(
    db: &Connection,
    thread: ThreadId,
    recipient: &SeatId,
    created: bool,
    modern: bool,
) -> Result<CommandResult, ApiError> {
    if !modern {
        return Ok(CommandResult::ThreadCreated(thread));
    }
    use crate::protocol::{
        commands::InvitationConfirmation,
        results::{DirectThreadEnsureOutcome, DirectThreadEnsured, DirectThreadInvitation},
    };
    let membership = crate::store::service_substrate::effective_membership(db, &thread, recipient)?;
    let (outcome, invitation) = if membership.native_state == Some("joined") {
        (DirectThreadEnsureOutcome::AlreadyJoined, None)
    } else if membership.native_state == Some("invited") {
        let (id,episode,confirmation,deadline,inviter):(String,i64,Option<String>,i64,Option<String>)=db.query_row(
            "SELECT i.id,i.episode,i.confirmation,i.deadline_at,i.inviter_seat_id FROM invitations i JOIN memberships m ON m.thread_id=i.thread_id AND m.seat_id=i.seat_id AND m.episode=i.episode WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending'",
            params![thread.as_str(),recipient.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(store_error)?;
        let confirmation = match confirmation.as_deref() {
            Some("lazy") => InvitationConfirmation::Lazy,
            Some("require_ack") | None => InvitationConfirmation::RequireAck,
            _ => {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "invalid direct invitation confirmation",
                ));
            }
        };
        (
            if created {
                DirectThreadEnsureOutcome::Created
            } else {
                DirectThreadEnsureOutcome::ExistingPending
            },
            Some(DirectThreadInvitation {
                id: InvitationId::new(id),
                episode: u64::try_from(episode).map_err(|_| {
                    api_error(ErrorCode::StoreCorrupt, "invalid direct invitation episode")
                })?,
                confirmation,
                effective_deadline_at: if confirmation == InvitationConfirmation::Lazy {
                    None
                } else {
                    Some(UtcMillis(deadline))
                },
                inviter: inviter.map(SeatId::new),
            }),
        )
    } else {
        return Err(api_error(
            ErrorCode::Conflict,
            "direct participant no longer invited or joined",
        ));
    };
    Ok(CommandResult::DirectThreadEnsured(DirectThreadEnsured {
        thread,
        recipient: recipient.clone(),
        outcome,
        invitation,
    }))
}
