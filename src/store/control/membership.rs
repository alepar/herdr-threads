//! Thread membership.

use super::{cooperative_payload_hash, decide_accountable, guard_pair_member, validate_actor};
use crate::protocol::authority::{MutationPermit, ObligationRef};
use crate::protocol::commands::Leave;
use crate::protocol::ids::{SeatId, ThreadId};
use crate::protocol::results::{ApiError, CommandResult, ErrorCode};
use crate::protocol::time::CallBudget;
use crate::store::connection::{StoreContext, api_error, store_error};
use crate::store::schema::{self, EventInput};
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

pub(super) fn bump_member_directory(
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
            let episode = crate::store::service_controls::next_invitation_episode(
                tx,
                &command.thread,
                &caller,
            )?;
            let instance = &command.claim.instance;
            let seq = schema::next_decision_seq(tx, instance)?;
            tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state,joined_at,home_transition,last_joined_n) VALUES (?1,?2,?3,'joined','joined',?4,1,1) ON CONFLICT(thread_id,seat_id) DO UPDATE SET episode=excluded.episode,state='joined',voluntary_state='joined',joined_at=excluded.joined_at,left_at=NULL,home_transition=memberships.home_transition+1,last_joined_n=memberships.home_transition+1",
                params![command.thread.as_str(),caller.as_str(),episode,decision.utc.0]).map_err(store_error)?;
            tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,?3,?4)",
                params![command.thread.as_str(),caller.as_str(),episode,seq as i64]).map_err(store_error)?;
            crate::store::read_cursors::init_for_join(tx, &caller, &command.thread)?;
            let payload = serde_json::json!({"action":"join", "seat":caller.as_str(),
                "generation":actor.binding_generation,"observation":actor.provenance})
            .to_string();
            schema::append_attributed_event_once_with_decision_seq(
                tx,
                EventInput {
                    thread: &command.thread,
                    key: &format!("join:{}:{}", command.operation.as_str(), caller.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: None,
                },
                seq,
                crate::protocol::service::EventAuthor::Native(caller.clone()),
            )?;
            schema::bump_membership_revision(tx, &command.thread)?;
            schema::bump_filter_revision(tx, instance, "directory", "all")?;
            bump_member_directory(tx, instance, &caller)?;
            schema::bump_filter_revision(tx, instance, "inbox", caller.as_str())?;
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
            let changed = tx.execute("UPDATE memberships SET state='left',voluntary_state='left',left_at=?1 WHERE thread_id=?2 AND seat_id=?3 AND state='joined'", params![decision.utc.0, command.thread.as_str(), caller.as_str()]).map_err(store_error)?;
            if changed != 0 {
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM seats WHERE id=?1",
                        [caller.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                let left_seq = schema::next_decision_seq(tx, &instance)?;
                tx.execute("UPDATE membership_intervals SET left_seq=?1 WHERE thread_id=?2 AND seat_id=?3 AND left_seq IS NULL",
                    params![left_seq as i64,command.thread.as_str(),caller.as_str()]).map_err(store_error)?;
                let payload =
                    serde_json::json!({"action":"leave", "seat":caller.as_str()}).to_string();
                schema::append_event_once_with_decision_seq(
                    tx,
                    EventInput {
                        thread: &command.thread,
                        key: &format!("leave:{}:{}", command.operation.as_str(), caller.as_str()),
                        kind: "info",
                        payload_json: &payload,
                        decision_at: decision.utc,
                        source_message: None,
                        source_invitation: None,
                    },
                    left_seq,
                )?;
                schema::bump_membership_revision(tx, &command.thread)?;
                schema::bump_filter_revision(tx, &instance, "directory", "all")?;
                bump_member_directory(tx, &instance, &caller)?;
                schema::bump_filter_revision(tx, &instance, "inbox", caller.as_str())?;
            }
            Ok(CommandResult::Left(command.thread.clone()))
        },
    )
}
