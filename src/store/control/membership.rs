//! Thread membership: the `join` and `leave` commands and the materializers of
//! the membership fact kinds (`membership.joined`, `membership.left`).
//!
//! Responsibility
//! - Owns: the commands `join` and `leave` (decide under A2, then author the
//!   transition through [`FactRoute`]), the one writer of `memberships` and
//!   `membership_intervals` for a joined or left transition
//!   ([`materialize_membership`], and [`materialize_joined`] for an accepted
//!   invitation), and the one `already_joined` rule
//!   ([`settle_superseded_invitations`]).
//! - Must not: write a membership transition outside a materializer, nor
//!   reject a fact because of the local membership view (receivers apply; only
//!   the origin-authority check rejects).
//! - Role: `sync::facts::membership` handlers and the local-only route call the
//!   materializers; `invitation.created` and `invitation.answered` (ht-5fr.4.10)
//!   call [`settle_superseded_invitations`] and [`materialize_joined`].

use super::thread::{has_creating_fact, local_instance};
use super::{cooperative_payload_hash, decide_accountable, guard_pair_member, validate_actor};
use crate::protocol::authority::{MutationPermit, ObligationRef};
use crate::protocol::commands::Leave;
use crate::protocol::ids::{SeatId, ThreadId};
use crate::protocol::results::{ApiError, CommandResult, ErrorCode};
use crate::protocol::time::CallBudget;
use crate::store::attribution;
use crate::store::connection::{DecisionInstant, StoreContext, api_error, store_error};
use crate::store::materialize::{FactRoute, MaterializeInput};
use crate::store::schema::{self, EventInput};
use crate::sync::facts::membership::{MembershipJoined, MembershipLeft, MembershipPayload};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

pub(super) fn joined_in_thread(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<bool, ApiError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM memberships m JOIN seats s ON s.id=m.seat_id \
        WHERE m.thread_id=?1 AND m.seat_id=?2 AND m.state='joined' AND s.state!='retired')",
        params![thread.as_str(), seat.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

pub(crate) fn bump_member_directory(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
) -> Result<(), ApiError> {
    schema::bump_filter_revision(
        tx,
        instance,
        "directory",
        &format!("member:{}", seat.as_str()),
    )
}

/// Voluntary self-enrollment, distinct from invitation or requirement consent.
pub fn join(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &crate::protocol::commands::Join,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("join", command)?;
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
            guard_pair_member(tx, &command.thread, &caller)?;
            let instance = validate_actor(tx, &caller, command.claim.target.as_str())?;
            let archived: Option<bool> = tx
                .query_row(
                    "SELECT archived FROM threads WHERE id=?1 AND instance_id=?2",
                    params![command.thread.as_str(), instance],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            match archived {
                None => {
                    return Err(api_error(
                        ErrorCode::NotFound,
                        "thread not found in caller instance",
                    ));
                }
                Some(true) => {
                    return Err(api_error(
                        ErrorCode::Conflict,
                        "thread is archived; ask a joined member or service owner to reopen it",
                    ));
                }
                Some(false) => {}
            }
            if let Some(required) =
                crate::store::service_substrate::current_requirement(tx, &command.thread, &caller)?
                && required.state == crate::protocol::service::RequirementState::Pending
            {
                return Err(api_error(
                    ErrorCode::MembershipRequired,
                    "pending required invitation; reread thread participants and use accept-required with its exact revision",
                ));
            }
            let pending: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=i.id))",
                params![command.thread.as_str(), caller.as_str()], |r| r.get(0),
            ).map_err(store_error)?;
            if pending {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "pending invitation; use accept THREAD or reject its exact invitation before joining",
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
            if joined_in_thread(tx, &command.thread, &caller)? {
                return Ok(CommandResult::Joined(command.thread.clone()));
            }
            let n = next_transition(tx, &command.thread, &caller)?;
            attribution::stage(
                tx,
                &attribution::join_key(command.thread.as_str(), caller.as_str(), n),
                &actor,
                &actor.observation(decision.utc.0),
            )?;
            author_membership(
                tx,
                context,
                decision,
                Transition::Joined,
                &MembershipPayload {
                    thread: command.thread.clone(),
                    seat: caller.clone(),
                    n,
                    at: decision.utc.0,
                },
            )?;
            Ok(CommandResult::Joined(command.thread.clone()))
        },
    )
}

pub fn leave(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &Leave,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("leave", command)?;
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
            validate_actor(tx, &caller, target)?;
            if let Some(required) =
                crate::store::service_substrate::current_requirement(tx, &command.thread, &caller)?
                && required.state == crate::protocol::service::RequirementState::Accepted
            {
                return Err(api_error(
                    ErrorCode::MembershipRequired,
                    format!(
                        "thread {} is required by {}; ask the service owner to release requirement {}",
                        command.thread.as_str(),
                        required.issuer.as_str(),
                        required.requirement.as_str()
                    ),
                ));
            }
            let state: Option<String> = tx
                .query_row(
                    "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                    params![command.thread.as_str(), caller.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            if !matches!(state.as_deref(), Some("joined" | "left")) {
                return Err(api_error(ErrorCode::Conflict, "caller is not joined"));
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
            if membership_row(tx, &command.thread, &caller)?
                .is_some_and(|row| row.state == "joined")
            {
                let n = next_transition(tx, &command.thread, &caller)?;
                author_membership(
                    tx,
                    context,
                    decision,
                    Transition::Left,
                    &MembershipPayload {
                        thread: command.thread.clone(),
                        seat: caller.clone(),
                        n,
                        at: decision.utc.0,
                    },
                )?;
            }
            Ok(CommandResult::Left(command.thread.clone()))
        },
    )
}

/// Which way a membership fact moves the seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Joined,
    Left,
}

/// The replicated counters of one membership row.
struct MembershipRow {
    state: String,
    home_transition: i64,
    last_joined_n: i64,
}

fn membership_row(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<Option<MembershipRow>, ApiError> {
    tx.query_row(
        "SELECT state,home_transition,last_joined_n FROM memberships WHERE thread_id=?1 AND seat_id=?2",
        params![thread.as_str(), seat.as_str()],
        |r| {
            Ok(MembershipRow {
                state: r.get(0)?,
                home_transition: r.get(1)?,
                last_joined_n: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(store_error)
}

/// The ordinal of the seat's next transition in the thread
/// (`home_transition + 1`; 1 without a row). A command on the seat's home node
/// computes it inside its transaction.
fn next_transition(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<u64, ApiError> {
    let current = membership_row(tx, thread, seat)?.map_or(0, |row| row.home_transition);
    u64::try_from(current)
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, "membership ordinal exhausted"))
}

/// The route of a membership transition in `thread`. A journaled thread no
/// fact created yet (transitional: a pair thread `ensure_direct_thread` still
/// inserts directly until ht-5fr.4.10, or a fixture whose journal never
/// opened) would park the fact on its `needs`, so it keeps the node-local
/// write it always had.
fn membership_route(tx: &Transaction<'_>, thread: &ThreadId) -> Result<FactRoute, ApiError> {
    let route = FactRoute::for_thread(tx, thread)?;
    if route == FactRoute::Journal && !has_creating_fact(tx, thread)? {
        return Ok(FactRoute::LocalOnly);
    }
    Ok(route)
}

/// Authors one membership transition for a command: through the journal for a
/// journaled thread, as a node-local write for any other.
pub(crate) fn author_membership(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    transition: Transition,
    payload: &MembershipPayload,
) -> Result<(), ApiError> {
    let route = membership_route(tx, &payload.thread)?;
    let local = |tx: &Transaction<'_>, m: &MaterializeInput<'_>, p: &MembershipPayload| {
        materialize_membership(tx, m, transition, p)
    };
    match transition {
        Transition::Joined => {
            route.emit::<MembershipJoined>(tx, ctx, decision, payload, None, local)?;
        }
        Transition::Left => {
            route.emit::<MembershipLeft>(tx, ctx, decision, payload, None, local)?;
        }
    }
    Ok(())
}

/// Materializes `membership.joined` / `membership.left` (fact-kinds design
/// section 7). `n <= home_transition` is a no-op, so a duplicate or re-issued
/// fact changes nothing and appends no second event. See [`materialize_joined`]
/// and the left arm below for the rows each transition writes.
pub fn materialize_membership(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    transition: Transition,
    payload: &MembershipPayload,
) -> Result<(), ApiError> {
    match transition {
        Transition::Joined => materialize_joined(
            tx,
            m,
            &JoinedTransition {
                thread: &payload.thread,
                seat: &payload.seat,
                n: payload.n,
                at: payload.at,
                join_event: true,
            },
        ),
        Transition::Left => materialize_left(tx, m, payload),
    }
}

/// A transition of a seat into `joined`.
pub(crate) struct JoinedTransition<'a> {
    pub thread: &'a ThreadId,
    pub seat: &'a SeatId,
    pub n: u64,
    pub at: i64,
    /// Whether to append the derived `join` event (a `membership.joined`
    /// does; an accepted invitation appends its own `accept` event).
    pub join_event: bool,
}

fn ordinal(n: u64) -> Result<i64, ApiError> {
    i64::try_from(n)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "membership ordinal out of range"))
}

/// The transition into `joined` with ordinal `t.n`: with `n > home_transition`
/// it sets `state`/`voluntary_state` to `joined`, `joined_at`, clears `left_at`,
/// records `home_transition = n` and `last_joined_n = max(last_joined_n, n)`,
/// opens a new local episode and `membership_intervals` row at the entry's
/// `decision_seq` (the creator's first join: at the `create:` event's, which
/// appends no `join` event), initializes the read cursors of a local seat, appends the
/// `join:<thread>:<seat>:<n>` event (attribution from `attribution::take`),
/// settles superseded invitations and bumps the revisions and filters.
///
/// A `retired` row records `home_transition` only (retirement is terminal). A
/// seat already `joined` records the counters and settles invitations without
/// a second interval or event, which only a forked log can produce.
pub(crate) fn materialize_joined(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    t: &JoinedTransition<'_>,
) -> Result<(), ApiError> {
    let n = ordinal(t.n)?;
    let row = membership_row(tx, t.thread, t.seat)?;
    let (state, home_transition, last_joined_n) = row.as_ref().map_or(("absent", 0, 0), |r| {
        (r.state.as_str(), r.home_transition, r.last_joined_n)
    });
    if n <= home_transition {
        return Ok(());
    }
    let attribution = if t.join_event {
        Some(attribution::take(
            tx,
            m,
            &attribution::join_key(t.thread.as_str(), t.seat.as_str(), t.n),
        )?)
    } else {
        None
    };
    if state == "retired" {
        tx.execute(
            "UPDATE memberships SET home_transition=?1 WHERE thread_id=?2 AND seat_id=?3",
            params![n, t.thread.as_str(), t.seat.as_str()],
        )
        .map_err(store_error)?;
        return Ok(());
    }
    let last_joined_n = last_joined_n.max(n);
    let creator_seq = if t.join_event && t.n == 1 {
        creator_create_seq(tx, t.thread, t.seat)?
    } else {
        None
    };
    if state == "joined" {
        tx.execute(
            "UPDATE memberships SET home_transition=?1,last_joined_n=?2 WHERE thread_id=?3 AND seat_id=?4",
            params![n, last_joined_n, t.thread.as_str(), t.seat.as_str()],
        )
        .map_err(store_error)?;
    } else {
        let episode =
            crate::store::service_controls::next_invitation_episode(tx, t.thread, t.seat)?;
        if row.is_some() {
            tx.execute(
                "UPDATE memberships SET state='joined',voluntary_state='joined',episode=?1,joined_at=?2,left_at=NULL,home_transition=?3,last_joined_n=?4 WHERE thread_id=?5 AND seat_id=?6",
                params![episode, t.at, n, last_joined_n, t.thread.as_str(), t.seat.as_str()],
            )
            .map_err(store_error)?;
        } else {
            tx.execute(
                "INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state,joined_at,home_transition,last_joined_n) VALUES (?1,?2,?3,'joined','joined',?4,?5,?6)",
                params![t.thread.as_str(), t.seat.as_str(), episode, t.at, n, last_joined_n],
            )
            .map_err(store_error)?;
        }
        tx.execute(
            "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,?3,?4)",
            params![
                t.thread.as_str(),
                t.seat.as_str(),
                episode,
                creator_seq.unwrap_or(m.decision_seq as i64)
            ],
        )
        .map_err(store_error)?;
        if crate::store::seats::is_local_seat(tx, t.seat)? {
            crate::store::read_cursors::init_for_join(tx, t.seat, t.thread)?;
        }
        // The creator's own first join is implied by the `create:<thread>`
        // event, which names the actor: it appends no second event.
        if let Some(attribution) = attribution.filter(|_| creator_seq.is_none()) {
            let observed: Option<String> =
                serde_json::from_str::<serde_json::Value>(&attribution.observation)
                    .ok()
                    .and_then(|v| v.get("provenance")?.as_str().map(str::to_owned));
            let payload = serde_json::json!({"action":"join","seat":t.seat.as_str(),
                "generation":attribution.generation,"observation":observed.unwrap_or_default()})
            .to_string();
            schema::append_attributed_event_once_with_decision_seq(
                tx,
                EventInput {
                    thread: t.thread,
                    key: &format!("join:{}:{}:{}", t.thread.as_str(), t.seat.as_str(), t.n),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: m.decision.utc,
                    source_message: None,
                    source_invitation: None,
                },
                m.decision_seq,
                crate::protocol::service::EventAuthor::Native(t.seat.clone()),
            )?;
        }
        bump_membership(tx, t.thread, t.seat)?;
    }
    settle_superseded_invitations(tx, t.thread, t.seat, t.n)?;
    Ok(())
}

/// The `decision_seq` of the `create:<thread>` event when it names `seat` as
/// its actor, else `None`. A thread's creator joins at that event's position
/// (so it sees the creation) and the join appends no event of its own. The
/// event is replicated state every node holds before `membership.joined`
/// applies, so every node decides alike.
fn creator_create_seq(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<Option<i64>, ApiError> {
    tx.query_row(
        "SELECT decision_seq FROM messages WHERE event_key=?1 AND json_extract(event_json,'$.actor_seat')=?2",
        params![format!("create:{}", thread.as_str()), seat.as_str()],
        |r| r.get(0),
    )
    .optional()
    .map_err(store_error)
}

fn bump_membership(tx: &Transaction<'_>, thread: &ThreadId, seat: &SeatId) -> Result<(), ApiError> {
    let instance = local_instance(tx)?;
    schema::bump_membership_revision(tx, thread)?;
    schema::bump_filter_revision(tx, &instance, "directory", "all")?;
    bump_member_directory(tx, &instance, seat)?;
    schema::bump_filter_revision(tx, &instance, "inbox", seat.as_str())
}

/// The transition into `left`: with `n > home_transition` a `joined` seat
/// becomes `left`, its open interval closes at the entry's `decision_seq`, the
/// `leave:<thread>:<seat>:<n>` event is appended and the revisions and filters
/// bump. A seat that is not `joined` (and a `retired` row) records
/// `home_transition` only.
fn materialize_left(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &MembershipPayload,
) -> Result<(), ApiError> {
    let n = ordinal(payload.n)?;
    let (thread, seat) = (&payload.thread, &payload.seat);
    let row = membership_row(tx, thread, seat)?;
    let state = row.as_ref().map_or("absent", |r| r.state.as_str());
    if row.as_ref().is_some_and(|r| n <= r.home_transition) {
        return Ok(());
    }
    if state != "joined" {
        if row.is_some() {
            tx.execute(
                "UPDATE memberships SET home_transition=?1 WHERE thread_id=?2 AND seat_id=?3",
                params![n, thread.as_str(), seat.as_str()],
            )
            .map_err(store_error)?;
        } else {
            // A leave whose join this store never applied: keep the ordinal so
            // an older join cannot resurrect the seat.
            let episode =
                crate::store::service_controls::next_invitation_episode(tx, thread, seat)?;
            tx.execute(
                "INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state,left_at,home_transition) VALUES (?1,?2,?3,'left','left',?4,?5)",
                params![thread.as_str(), seat.as_str(), episode, payload.at, n],
            )
            .map_err(store_error)?;
        }
        return Ok(());
    }
    tx.execute(
        "UPDATE memberships SET state='left',voluntary_state='left',left_at=?1,home_transition=?2 WHERE thread_id=?3 AND seat_id=?4",
        params![payload.at, n, thread.as_str(), seat.as_str()],
    )
    .map_err(store_error)?;
    tx.execute(
        "UPDATE membership_intervals SET left_seq=?1 WHERE thread_id=?2 AND seat_id=?3 AND left_seq IS NULL",
        params![m.decision_seq as i64, thread.as_str(), seat.as_str()],
    )
    .map_err(store_error)?;
    let event = serde_json::json!({"action":"leave","seat":seat.as_str()}).to_string();
    schema::append_event_once_with_decision_seq(
        tx,
        EventInput {
            thread,
            key: &format!("leave:{}:{}:{}", thread.as_str(), seat.as_str(), payload.n),
            kind: "info",
            payload_json: &event,
            decision_at: m.decision.utc,
            source_message: None,
            source_invitation: None,
        },
        m.decision_seq,
    )?;
    bump_membership(tx, thread, seat)
}

/// The one `already_joined` rule: every `pending` invitation of `seat` in
/// `thread` whose `invitee_n` is below `n` (the invitee joined after the
/// inviter's view) becomes `already_joined`. It reads ordinals only, never the
/// current membership `state`, so whichever of "invitee joined" and
/// "invitation created" applies first the invitation ends the same, and a join
/// followed by a leave supersedes it in every order too.
///
/// Responsibility
/// - Owns: this settlement. It is not an answer: it wakes nobody, records no
///   overdue and keeps `deadline_at` as stored.
/// - Run by: `membership.joined` ([`materialize_joined`], so also an accepted
///   invitation) and, for an invitation that applies after the join,
///   `invitation.created` (ht-5fr.4.10) with the same `last_joined_n >
///   invitee_n` comparison.
/// - Must not: settle by membership state, or decide for a rejected or
///   accepted invitation (only `pending` rows move).
///
/// Returns how many invitations it settled.
pub fn settle_superseded_invitations(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    seat: &SeatId,
    n: u64,
) -> Result<usize, ApiError> {
    tx.execute(
        "UPDATE invitations SET state='already_joined' WHERE thread_id=?1 AND seat_id=?2 AND state='pending' AND invitee_n<?3",
        params![thread.as_str(), seat.as_str(), ordinal(n)?],
    )
    .map_err(store_error)
}
