//! The target's single staging transaction of a remote handoff (spec §3.2 step 2, §3.3,
//! TRUST-POLICY N15).
//!
//! Responsibility (PeerHandoffStage)
//! - Owns: authoring the three facts a handoff request may cause, in one transaction and
//!   idempotent on the request id: the pair's `thread.direct` (only when the pair is absent), the
//!   recipient seat's own `membership.joined` and the one `message.sent` restating the request's
//!   sender block; recording `handoff_thread` and `handoff_message` on the `peer_requests` row,
//!   and optionally the hc launch gate's record in the same commit.
//! - Hides: how the pair is looked up, which membership the target owes and how the payload is
//!   built (only [`SenderBlock::expected_payload`]; never by hand).
//! - Must not: read a caller claim (the `peer_requests` row is the authority), author an
//!   invitation, a receipt settlement or the sender seat's membership (its home node does, N4),
//!   or launch anything.
//!
//! Core and harness-neutral.

use rusqlite::Transaction;

use crate::protocol::ids::{MessageId, SeatId, ThreadId, direct_thread_id};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::connection::{DecisionInstant, StoreContext, api_error, store_error};
use crate::store::control::Transition;
use crate::store::materialize::FactRoute;
use crate::store::{control, messages, peer_requests, seats};
use crate::sync::facts::message::MessageSent;
use crate::sync::facts::thread::ThreadDirectPayload;
use crate::sync::frame::LaunchPlan;

/// The hc launch gate's record, written in the staging commit so the gate opens with its message.
pub struct GateRecord<'a> {
    /// The `CreatedTarget` JSON.
    pub created: &'a str,
    /// The created terminal; the row's `pane`.
    pub terminal: &'a str,
}

/// What staging authored (or found already authored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub thread: ThreadId,
    pub message: MessageId,
    /// The requesting node, for the audit line.
    pub sender_node: String,
    /// False when a repeat found the facts already staged and wrote nothing.
    pub fresh: bool,
}

fn refuse(detail: &str) -> ApiError {
    api_error(ErrorCode::Conflict, detail)
}

/// What the launch child of handoff request `request` launches: the stored plan, the frozen
/// prompt and the row's state. Read-only; only a row carrying a signed handoff block answers.
pub fn request_view(
    conn: &rusqlite::Connection,
    request: &str,
) -> Result<crate::protocol::results::CommandResult, ApiError> {
    let row = peer_requests::get(conn, request)
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "unknown peer request"))?;
    let plan: LaunchPlan = serde_json::from_str(&row.plan)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "peer request plan unreadable"))?;
    if plan.handoff().is_none() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "peer request carries no handoff",
        ));
    }
    Ok(crate::protocol::results::CommandResult::PeerRequestView(
        Box::new(crate::protocol::results::PeerRequestView {
            plan,
            handoff_prompt: row.handoff_prompt,
            state: row.state,
        }),
    ))
}

/// Stages request `request` for `seat` in `tx`. Any failure aborts the transaction with no fact;
/// the caller then records the request plain `failed`.
pub fn stage(
    tx: &Transaction<'_>,
    context: &StoreContext,
    decision: DecisionInstant,
    request: &str,
    seat: &SeatId,
    gate: Option<GateRecord<'_>>,
) -> Result<Staged, ApiError> {
    let row = peer_requests::handoff_view(tx, request)
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "unknown peer request"))?;
    // The row's own seat when it names one: checked first, so a repeat naming another seat never
    // gets this request's facts.
    if row
        .seat
        .as_deref()
        .is_some_and(|recorded| recorded != seat.as_str())
    {
        return Err(refuse("peer request names another seat"));
    }
    // Idempotent on the request id: the facts stand whatever the row's state is now.
    if let (Some(thread), Some(message)) = (&row.handoff_thread, &row.handoff_message) {
        return Ok(Staged {
            thread: ThreadId::new(thread.clone()),
            message: MessageId::new(message.clone()),
            sender_node: row.node_uuid,
            fresh: false,
        });
    }
    let plan: LaunchPlan = serde_json::from_str(&row.plan)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "peer request plan unreadable"))?;
    let block = plan
        .handoff()
        .ok_or_else(|| api_error(ErrorCode::InvalidRequest, "peer request carries no handoff"))?;
    if row.state != "running" {
        return Err(refuse("peer request is no longer running"));
    }
    // The seat is still the target's: a local seat that is not retired.
    let local: bool = tx
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND {} AND state!='retired')",
                seats::LOCAL_SEAT
            ),
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !local {
        return Err(refuse("target seat is not a live seat of this node"));
    }

    let payload = block
        .expected_payload(seat, decision.utc.0)
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e.to_string()))?;
    debug_assert!(block.restates(&payload));
    let thread = payload.thread.clone();
    let author = payload.author.clone();

    // The pair's thread id is a pure function of its seats, so an existing pair is found by id
    // (as `resolve_direct_pair` does) and reused whatever created it.
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1)",
            [thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if exists {
        control::direct_pair(tx, &thread)?
            .ok_or_else(|| refuse("handoff thread is not a direct pair"))?;
    } else {
        let (lo, hi) = if &author < seat {
            (author.clone(), seat.clone())
        } else {
            (seat.clone(), author.clone())
        };
        debug_assert_eq!(direct_thread_id(&lo, &hi).ok().as_ref(), Some(&thread));
        control::author_thread_direct(
            tx,
            context,
            decision,
            &ThreadDirectPayload {
                thread: thread.clone(),
                seat_lo: lo,
                seat_hi: hi,
                creator: Some(author.clone()),
            },
        )?;
    }
    // Only the recipient's join: the sender seat's home node authors its own (N4). The ordinary
    // pair auto-join rule, so a seat that already holds a membership row owes nothing.
    for join in
        control::auto_join_local_pair_seats(tx, &thread, &[seat], Some(&author), decision.utc.0)?
    {
        control::author_membership(tx, context, decision, Transition::Joined, &join)?;
    }
    let route = messages::send_route(tx, context, &thread)?;
    if route == FactRoute::Journal {
        payload.validate_for_author()?;
    }
    route.emit::<MessageSent>(
        tx,
        context,
        decision,
        &payload,
        None,
        messages::materialize_message,
    )?;

    if !peer_requests::record_handoff(
        tx,
        request,
        thread.as_str(),
        payload.message.as_str(),
        decision.utc.0,
    )
    .map_err(store_error)?
    {
        return Err(refuse("peer request is no longer running"));
    }
    if let Some(gate) = gate
        && !peer_requests::record_gate_launch(
            tx,
            request,
            gate.created,
            gate.terminal,
            seat.as_str(),
            decision.utc.0,
        )
        .map_err(store_error)?
    {
        return Err(refuse("launch gate record is not open"));
    }
    Ok(Staged {
        thread,
        message: payload.message,
        sender_node: row.node_uuid,
        fresh: true,
    })
}
