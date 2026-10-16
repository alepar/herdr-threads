//! Direct-pair channels.

use super::{
    Transition, author_membership, cooperative_payload_hash, decide_accountable, validate_actor,
};
use crate::protocol::authority::{MutationPermit, ObligationRef};
use crate::protocol::ids::{InvitationId, SeatId, ThreadId, direct_thread_id};
use crate::protocol::results::{ApiError, CommandResult, ErrorCode};
use crate::protocol::time::{CallBudget, UtcMillis};
use crate::store::attribution;
use crate::store::connection::{DecisionInstant, StoreContext, api_error, store_error};
use crate::store::materialize::{FactRoute, local_only};
use crate::store::schema;
use crate::sync::facts::membership::MembershipPayload;
use crate::sync::facts::thread::{ThreadDirect, ThreadDirectPayload};
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
    let peer_ok = match peer.native_state {
        Some("joined" | "invited") => true,
        // A peer with no row yet whose home node owes the auto-join.
        None => pending_auto_join(db, thread, recipient)?,
        Some(_) => false,
    };
    if !peer_ok {
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
                context,
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
pub fn resolve_direct_pair(
    tx: &Transaction<'_>,
    context: &StoreContext,
    decision: DecisionInstant,
    claim: &crate::protocol::authority::CallerClaim,
    recipient: &SeatId,
    creation_policy: Option<&crate::protocol::commands::EnsureCreationPolicy>,
    _installation_default_ms: Option<u64>,
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
    // The pair's thread ID is a pure function of its two seats, so the lookup
    // is by thread ID and two nodes that open the same pair name one thread.
    let thread =
        direct_thread_id(a, b).map_err(|e| api_error(ErrorCode::InvalidRequest, e.to_string()))?;
    let existing: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
            params![thread.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if existing {
        direct_pair(tx, &thread)?;
        require_pair_lifecycle(tx, &thread, caller, recipient)?;
        return Ok((thread, false));
    }
    // The creation policy is shape-validated only: a new pair records no
    // invitation, so there is no deadline or confirmation to compute.
    if let Some(policy) = creation_policy {
        policy
            .validate()
            .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    }
    // Three facts, in this order: the pair's thread, the caller's membership
    // (the creating join: its event is the thread's `create:` event) and, only
    // for a recipient seat this node hosts, the recipient's auto-join. A
    // remote recipient's home node authors its own auto-join when it applies
    // the `thread.direct`. A crossed create on another node authors the same
    // `d...` thread; each side's seats then hold their rows and skip it.
    let direct = ThreadDirectPayload {
        thread: thread.clone(),
        seat_lo: a.clone(),
        seat_hi: b.clone(),
        creator: Some(caller.clone()),
    };
    if context.journal_open() {
        FactRoute::Journal.emit::<ThreadDirect>(
            tx,
            context,
            decision,
            &direct,
            None,
            |_, _, _| {
                Err(api_error(
                    ErrorCode::Unsupported,
                    "thread.direct is journaled",
                ))
            },
        )?;
    } else {
        // Only a store whose journal never opened (a fixture; the daemon opens
        // it before serving) gets here: the same materializer, written locally.
        let input = local_only(tx, decision)?;
        super::materialize_thread_direct(tx, &input, &direct)?;
    }
    author_membership(
        tx,
        context,
        decision,
        Transition::Joined,
        &MembershipPayload {
            thread: thread.clone(),
            seat: caller.clone(),
            n: 1,
            at: decision.utc.0,
            actor: None,
        },
    )?;
    for payload in
        auto_join_local_pair_seats(tx, &thread, &[recipient], Some(caller), decision.utc.0)?
    {
        author_membership(tx, context, decision, Transition::Joined, &payload)?;
    }
    Ok((thread, true))
}

/// The pair seats this node auto-joins now (reconcile section 2.1): local, not
/// retired, and with no `memberships` row in `thread`. Each one's
/// `direct_auto_join` attribution is staged under its join key, and its
/// `membership.joined{seat, n = 1, at, actor}` payload returned for the caller
/// to author (inline) or queue (`ApplyContext::follow_up`). `actor` is
/// `creator` for a seat other than the creator itself and `None` for the
/// creator (its join is the creating join, placed at the `create:` event).
pub(crate) fn auto_join_local_pair_seats(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    seats: &[&SeatId],
    creator: Option<&SeatId>,
    at: i64,
) -> Result<Vec<MembershipPayload>, ApiError> {
    let mut payloads = Vec::new();
    for &seat in seats {
        let eligible: bool = tx
            .query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND {} AND state!='retired') \
                     AND NOT EXISTS(SELECT 1 FROM memberships WHERE thread_id=?2 AND seat_id=?1)",
                    crate::store::seats::LOCAL_SEAT
                ),
                params![seat.as_str(), thread.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !eligible {
            continue;
        }
        attribution::stage_unbound(
            tx,
            &attribution::join_key(thread.as_str(), seat.as_str(), 1),
            &serde_json::json!({"provenance": "direct_auto_join", "decided_at": at}).to_string(),
        )?;
        payloads.push(MembershipPayload {
            thread: thread.clone(),
            seat: seat.clone(),
            n: 1,
            at,
            actor: creator.filter(|c| *c != seat).cloned(),
        });
    }
    Ok(payloads)
}

/// The auto-join payloads this node owes after applying a `thread.direct`
/// another node (or an earlier incarnation) authored: one per pair seat it
/// hosts that has no membership row yet. The handler queues them as follow-ups.
pub fn direct_auto_join_payloads(
    tx: &Transaction<'_>,
    m: &crate::store::materialize::MaterializeInput<'_>,
    payload: &ThreadDirectPayload,
) -> Result<Vec<MembershipPayload>, ApiError> {
    auto_join_local_pair_seats(
        tx,
        &payload.thread,
        &[&payload.seat_lo, &payload.seat_hi],
        payload.creator.as_ref(),
        m.decision.utc.0,
    )
}

/// Whether `seat` is a pair seat hosted by another node that has no row in
/// `thread` yet: its home node owes the auto-join and it has not arrived.
pub(crate) fn pending_auto_join(
    db: &Connection,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<bool, ApiError> {
    let Some((a, b)) = direct_pair(db, thread)? else {
        return Ok(false);
    };
    if seat != &a && seat != &b {
        return Ok(false);
    }
    db.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND NOT ({}) AND state!='retired') \
             AND NOT EXISTS(SELECT 1 FROM memberships WHERE thread_id=?2 AND seat_id=?1)",
            crate::store::seats::LOCAL_SEAT
        ),
        params![seat.as_str(), thread.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
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
    let (outcome, invitation) = if membership.native_state == Some("joined")
        || (membership.native_state.is_none() && pending_auto_join(db, &thread, recipient)?)
    {
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
