//! Thread lifecycle and metadata.

use super::{
    bump_member_directory, cooperative_payload_hash, decide_accountable, joined_in_thread,
    validate_actor,
};
use crate::protocol::authority::{MutationPermit, ObligationRef};
use crate::protocol::commands::{CreateThread, SetTopic, ThreadMutation};
use crate::protocol::ids::{ThreadId, prefix};
use crate::protocol::results::{ApiError, CommandResult, ErrorCode};
use crate::protocol::time::CallBudget;
use crate::store::connection::{StoreContext, api_error, store_error};
use crate::store::schema::{self, EventInput};
use rusqlite::{Connection, params};

fn set_archived(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &ThreadMutation,
    mut permit: MutationPermit,
    archived: bool,
) -> Result<CommandResult, ApiError> {
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let action = if archived { "archive" } else { "reopen" };
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash(action, command)?;
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
            if !joined_in_thread(tx, &command.thread, &caller)? {
                return Err(api_error(ErrorCode::Unauthorized, "caller is not joined"));
            }
            if crate::store::service_substrate::managed_owner(tx, &command.thread)?.is_some() {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "managed thread control belongs to service owner",
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
            let changed = tx.execute("UPDATE threads SET archived=?1,updated_at=?2,directory_revision=directory_revision+1 WHERE id=?3 AND archived!=?1",
                params![archived, decision.utc.0, command.thread.as_str()]).map_err(store_error)?;
            if changed != 0 {
                let payload =
                    serde_json::json!({"action":action, "actor_seat":caller.as_str()}).to_string();
                schema::append_event_once(
                    tx,
                    EventInput {
                        thread: &command.thread,
                        key: &format!("{}:{}", action, command.operation.as_str()),
                        kind: "info",
                        payload_json: &payload,
                        decision_at: decision.utc,
                        source_message: None,
                        source_invitation: None,
                    },
                )?;
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM seats WHERE id=?1",
                        [caller.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            }
            Ok(if archived {
                CommandResult::Archived(command.thread.clone())
            } else {
                CommandResult::Reopened(command.thread.clone())
            })
        },
    )
}

pub fn archive(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &ThreadMutation,
    permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    set_archived(context, conn, budget, command, permit, true)
}

pub fn reopen(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &ThreadMutation,
    permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    set_archived(context, conn, budget, command, permit, false)
}

pub fn set_topic(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &SetTopic,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    if command.topic.is_empty() || command.topic.len() > 1024 {
        return Err(api_error(ErrorCode::InvalidRequest, "invalid thread topic"));
    }
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("set_topic", command)?;
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
            if !joined_in_thread(tx, &command.thread, &caller)? {
                return Err(api_error(ErrorCode::Unauthorized, "caller is not joined"));
            }
            if crate::store::service_substrate::managed_owner(tx, &command.thread)?.is_some() {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "managed thread control belongs to service owner",
                ));
            }
            Ok(())
        },
        |tx, at| {
            decide_accountable(
                context,
                tx,
                at,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let old: String = tx
                .query_row(
                    "SELECT topic FROM threads WHERE id=?1",
                    [command.thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if old != command.topic {
                let changed = tx.execute("UPDATE threads SET topic=?1,topic_revision=topic_revision+1,updated_at=?2 WHERE id=?3 AND topic_revision<?4",
                    params![command.topic,at.utc.0,command.thread.as_str(),i64::MAX]).map_err(store_error)?;
                if changed != 1 {
                    return Err(api_error(
                        ErrorCode::SequenceExhausted,
                        "topic revision exhausted",
                    ));
                }
                let payload=serde_json::json!({"action":"set_topic","actor_seat":caller.as_str(),"topic":command.topic}).to_string();
                let (event, _) = schema::append_event_once(
                    tx,
                    EventInput {
                        thread: &command.thread,
                        key: &format!("set_topic:{}", command.operation.as_str()),
                        kind: "info",
                        payload_json: &payload,
                        decision_at: at.utc,
                        source_message: None,
                        source_invitation: None,
                    },
                )?;
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM threads WHERE id=?1",
                        [command.thread.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                let seq: i64 = tx
                    .query_row(
                        "SELECT decision_seq FROM messages WHERE id=?1",
                        [event.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                crate::store::invitation_content::record_context_change(
                    tx,
                    &instance,
                    &command.thread,
                    u64::try_from(seq).map_err(|_| {
                        api_error(
                            ErrorCode::StoreCorrupt,
                            "negative context decision sequence",
                        )
                    })?,
                )?;
                schema::bump_filter_revision(tx, &instance, "topic", command.thread.as_str())?;
                schema::bump_filter_revision(tx, &instance, "topic", "all")?;
                schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            }
            Ok(CommandResult::TopicChanged(command.thread.clone()))
        },
    )
}

pub fn set_thread_name(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &crate::protocol::commands::SetThreadName,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    if let Some(name) = &command.name {
        crate::protocol::commands::validate_thread_name(name)
            .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    }
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("set_thread_name", command)?;
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
            if !joined_in_thread(tx, &command.thread, &caller)? {
                return Err(api_error(ErrorCode::Unauthorized, "caller is not joined"));
            }
            if crate::store::service_substrate::managed_owner(tx, &command.thread)?.is_some() {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "managed thread control belongs to service owner",
                ));
            }
            Ok(())
        },
        |tx, at| {
            decide_accountable(
                context,
                tx,
                at,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let old: Option<String> = tx
                .query_row(
                    "SELECT name FROM threads WHERE id=?1",
                    [command.thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if old != command.name {
                tx.execute(
                    "UPDATE threads SET name=?1,updated_at=?2 WHERE id=?3",
                    params![command.name, at.utc.0, command.thread.as_str()],
                )
                .map_err(store_error)?;
                let payload=serde_json::json!({"action":"set_thread_name","actor_seat":caller.as_str(),"name":command.name}).to_string();
                schema::append_event_once(
                    tx,
                    EventInput {
                        thread: &command.thread,
                        key: &format!("set_thread_name:{}", command.operation.as_str()),
                        kind: "info",
                        payload_json: &payload,
                        decision_at: at.utc,
                        source_message: None,
                        source_invitation: None,
                    },
                )?;
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM threads WHERE id=?1",
                        [command.thread.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                schema::bump_filter_revision(tx, &instance, "directory", "all")?;
                schema::bump_filter_revision(tx, &instance, "directory", "name/all")?;
            }
            Ok(CommandResult::ThreadNameChanged(command.thread.clone()))
        },
    )
}

/// A new thread is born with exactly one joined member and a creation audit.
/// The caller-supplied claim only locates the durable seat; the permit is
/// consumed against a decision-time fence before any write.
pub fn create_thread(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &CreateThread,
    permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    create_thread_impl(context, conn, budget, command, permit, None)
}

/// Bootstrap CREATE seam. `canonical` must come from the daemon's independently
/// selected NodePaths; this adds no wire mode or runtime admission.
pub fn create_thread_in_namespace(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &CreateThread,
    permit: MutationPermit,
    canonical: &crate::protocol::handoff::HandoffNamespace,
) -> Result<CommandResult, ApiError> {
    create_thread_impl(context, conn, budget, command, permit, Some(canonical))
}

fn create_thread_impl(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &CreateThread,
    mut permit: MutationPermit,
    canonical: Option<&crate::protocol::handoff::HandoffNamespace>,
) -> Result<CommandResult, ApiError> {
    if command.topic.is_empty() || command.topic.len() > 1024 || command.goal.len() > 1024 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid thread topic or goal",
        ));
    }
    if let Some(name) = &command.name {
        crate::protocol::commands::validate_thread_name(name)
            .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    }
    if let Some(canonical) = canonical {
        permit = permit.with_bootstrap_child(
            canonical,
            &crate::protocol::commands::PermitMutation::CreateThread(command.clone()),
        );
    }
    let target = command.claim.target.as_str();
    let seat = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", seat.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("create_thread", command)?;
    let obligation = ObligationRef::CheckIn(seat.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            validate_actor(tx, &seat, target)?;
            Ok(())
        },
        |tx, decision| {
            decide_accountable(
                context,
                tx,
                decision,
                &mut permit,
                &command.claim,
                &seat,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if let Some(canonical) = canonical {
                crate::store::topology_handoff::validate_create_command(tx, canonical, command)?;
            } else {
                crate::store::topology_handoff::guard_unscoped_create(
                    tx,
                    &instance,
                    &scope,
                    command.operation.as_str(),
                )?;
            }
            let thread = ThreadId::new(crate::store::public_ids::fresh(
                tx,
                prefix::THREAD,
                &[("threads", "id", prefix::THREAD)],
            )?);
            let joined_seq = schema::next_decision_seq(tx, &instance)?;
            tx.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at, name) VALUES (?1,?2,?3,?4,?5,?5,?6)",
                params![thread.as_str(), instance, command.topic,command.goal,decision.utc.0,command.name]).map_err(store_error)?;
            // Attach protection in this exact CREATE transaction. Historical
            // pre-fence CREATE results are reconciled by Begin/import instead.
            if crate::store::handoff::installed(tx)? {
                if let Some(canonical) = canonical {
                    crate::store::topology_handoff::attach_created(
                        tx,
                        canonical,
                        &instance,
                        &scope,
                        command.operation.as_str(),
                        &thread,
                    )?;
                } else {
                    crate::store::handoff::attach_created(
                        tx,
                        &instance,
                        &scope,
                        command.operation.as_str(),
                        &thread,
                    )?;
                }
            }
            tx.execute("INSERT INTO memberships(thread_id, seat_id, state, joined_at) VALUES (?1,?2,'joined',?3)",
                params![thread.as_str(), seat.as_str(), decision.utc.0]).map_err(store_error)?;
            tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,1,?3)",
                params![thread.as_str(),seat.as_str(),joined_seq as i64]).map_err(store_error)?;
            crate::store::read_cursors::init_for_join(tx, &seat, &thread)?;
            let payload = serde_json::json!({"action":"create_thread", "goal":command.goal,
                "actor_seat":seat.as_str()})
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
            schema::bump_membership_revision(tx, &thread)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &seat)?;
            schema::bump_filter_revision(tx, &instance, "inbox", seat.as_str())?;
            Ok(CommandResult::ThreadCreated(thread))
        },
    )
}
