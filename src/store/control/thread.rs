//! Thread lifecycle and metadata: the commands, and the materializers of the
//! thread fact kinds (`thread.created`, `thread.register`, `thread.direct`).
//!
//! Responsibility
//! - Owns: the commands `create_thread*`, `set_topic`, `set_thread_name`,
//!   `archive` and `reopen` (decide under A2, then author the fact through
//!   [`FactRoute`]), and the one writer of the `threads` registers and of
//!   `direct_threads`: [`materialize_thread_created`], [`materialize_register`]
//!   and [`materialize_thread_direct`], which `sync::facts::thread` handlers
//!   and the local-only route both call.
//! - Must not: write `threads`/`thread_registers` outside a materializer, nor
//!   reject a fact because of the local archive, membership or topic view
//!   (receivers apply; only the origin-authority check rejects).
//! - Role: a new register field is one arm in [`materialize_register`].

use super::{
    bump_member_directory, cooperative_payload_hash, decide_accountable, joined_in_thread,
    validate_actor,
};
use crate::protocol::authority::{MutationPermit, ObligationRef};
use crate::protocol::commands::{CreateThread, SetTopic, ThreadMutation};
use crate::protocol::ids::{SeatId, ServiceAuthorId, ThreadId, prefix};
use crate::protocol::results::{ApiError, CommandResult, ErrorCode};
use crate::protocol::time::CallBudget;
use crate::store::connection::{DecisionInstant, StoreContext, api_error, store_error};
use crate::store::materialize::{FactRoute, MaterializeInput, local_only};
use crate::store::schema::{self, EventInput};
use crate::store::thread_registers::{self, RegisterOutcome};
use crate::sync::facts::payload::RegisterField;
use crate::sync::facts::thread::{
    ThreadCreated, ThreadCreatedPayload, ThreadDirectPayload, ThreadRegister, ThreadRegisterPayload,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// The thread a materializer inserts. `journaled = false` is a service-managed
/// thread (`service_controls::ensure_thread`): node-local, no registers, no
/// creation event.
pub struct NewThread<'a> {
    pub thread: &'a ThreadId,
    pub topic: &'a str,
    pub goal: &'a str,
    pub name: Option<&'a str>,
    pub creator: Option<&'a SeatId>,
    pub managed_owner: Option<&'a ServiceAuthorId>,
    pub journaled: bool,
}

fn local_instance(tx: &Transaction<'_>) -> Result<String, ApiError> {
    tx.query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .map_err(store_error)
}

/// Materializes `thread.created` (fact-kinds design section 5): inserts the
/// thread if absent, seeds the topic and name registers from the entry's
/// `(hlc, node)`, appends the `create:<thread>` event and bumps the directory
/// filter. A thread that exists already is a no-op (a re-issue copies the
/// payload, and a fresh ID cannot collide). The creator's membership is not
/// written here.
pub fn materialize_thread_created(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    new: &NewThread<'_>,
) -> Result<(), ApiError> {
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1)",
            [new.thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if exists {
        return Ok(());
    }
    let instance = local_instance(tx)?;
    tx.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,name,managed_owner_author_id,journaled) \
         VALUES (?1,?2,?3,?4,?5,?5,?6,?7,?8)",
        params![
            new.thread.as_str(),
            instance,
            new.topic,
            new.goal,
            m.origin_ms,
            new.name,
            new.managed_owner.map(ServiceAuthorId::as_str),
            new.journaled
        ],
    )
    .map_err(store_error)?;
    if new.journaled {
        // A fact carries its journal row; a store whose journal never opened
        // (a fixture) writes the thread without registers.
        if let Some(seq) = m.ingest_seq {
            thread_registers::upsert(
                tx,
                new.thread,
                thread_registers::RegisterField::Topic,
                Some(new.topic),
                m.origin_hlc,
                m.origin_node,
                seq,
            )
            .map_err(store_error)?;
            if let Some(name) = new.name {
                thread_registers::upsert(
                    tx,
                    new.thread,
                    thread_registers::RegisterField::Name,
                    Some(name),
                    m.origin_hlc,
                    m.origin_node,
                    seq,
                )
                .map_err(store_error)?;
            }
        }
        let mut payload = serde_json::json!({"action":"create_thread","goal":new.goal});
        if let Some(creator) = new.creator {
            payload["actor_seat"] = serde_json::json!(creator.as_str());
        }
        create_event(tx, m, new.thread, &payload.to_string())?;
    }
    schema::bump_filter_revision(tx, &instance, "directory", "all")?;
    Ok(())
}

fn create_event(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    thread: &ThreadId,
    payload_json: &str,
) -> Result<(), ApiError> {
    schema::append_event_once_with_decision_seq(
        tx,
        EventInput {
            thread,
            key: &format!("create:{}", thread.as_str()),
            kind: "info",
            payload_json,
            decision_at: m.decision.utc,
            source_message: None,
            source_invitation: None,
        },
        m.decision_seq,
    )?;
    Ok(())
}

fn store_field(field: RegisterField) -> thread_registers::RegisterField {
    match field {
        RegisterField::Topic => thread_registers::RegisterField::Topic,
        RegisterField::Name => thread_registers::RegisterField::Name,
        RegisterField::Archived => thread_registers::RegisterField::Archived,
    }
}

/// Materializes `thread.register` (section 5): a last-writer-wins write of one
/// register. A write that loses changes nothing and appends nothing. A winner
/// is copied to `threads` when the value differs, with the revision and filter
/// bumps the local command made before, and a derived event keyed by the
/// entry (`register:<thread>:<field>:<uuid8>:<counter>`) is appended.
///
/// A local-only input (a node-local thread) has no journal row and no
/// register: the value is applied directly.
pub fn materialize_register(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &ThreadRegisterPayload,
) -> Result<(), ApiError> {
    let invalid = |why: &str| api_error(ErrorCode::InvalidRequest, why);
    match (payload.field, payload.value.as_deref()) {
        (RegisterField::Topic, None | Some("")) => return Err(invalid("thread topic is empty")),
        (RegisterField::Archived, Some("0" | "1")) => {}
        (RegisterField::Archived, _) => return Err(invalid("archived register is 0 or 1")),
        _ => {}
    }
    let instance: String = tx
        .query_row(
            "SELECT instance_id FROM threads WHERE id=?1",
            [payload.thread.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "thread not found"))?;
    if let Some(seq) = m.ingest_seq {
        let outcome = thread_registers::upsert(
            tx,
            &payload.thread,
            store_field(payload.field),
            payload.value.as_deref(),
            m.origin_hlc,
            m.origin_node,
            seq,
        )
        .map_err(store_error)?;
        if outcome == RegisterOutcome::Lost {
            return Ok(());
        }
    }
    let thread = payload.thread.as_str();
    let (field, event_payload) = match payload.field {
        RegisterField::Topic => {
            let topic = payload.value.as_deref().unwrap_or_default();
            let old: String = tx
                .query_row("SELECT topic FROM threads WHERE id=?1", [thread], |r| {
                    r.get(0)
                })
                .map_err(store_error)?;
            if old == topic {
                return Ok(());
            }
            let changed = tx
                .execute(
                    "UPDATE threads SET topic=?1,topic_revision=topic_revision+1,updated_at=?2 \
                     WHERE id=?3 AND topic_revision<?4",
                    params![topic, m.origin_ms, thread, i64::MAX],
                )
                .map_err(store_error)?;
            if changed != 1 {
                return Err(api_error(
                    ErrorCode::SequenceExhausted,
                    "topic revision exhausted",
                ));
            }
            (
                "topic",
                serde_json::json!({"action":"set_topic","topic":topic}),
            )
        }
        RegisterField::Name => {
            let name = payload.value.as_deref();
            let old: Option<String> = tx
                .query_row("SELECT name FROM threads WHERE id=?1", [thread], |r| {
                    r.get(0)
                })
                .map_err(store_error)?;
            if old.as_deref() == name {
                return Ok(());
            }
            tx.execute(
                "UPDATE threads SET name=?1,updated_at=?2 WHERE id=?3",
                params![name, m.origin_ms, thread],
            )
            .map_err(store_error)?;
            (
                "name",
                serde_json::json!({"action":"set_thread_name","name":name}),
            )
        }
        RegisterField::Archived => {
            let archived = payload.value.as_deref() == Some("1");
            let changed = tx
                .execute(
                    "UPDATE threads SET archived=?1,updated_at=?2,directory_revision=directory_revision+1 \
                     WHERE id=?3 AND archived!=?1",
                    params![archived, m.origin_ms, thread],
                )
                .map_err(store_error)?;
            if changed == 0 {
                return Ok(());
            }
            (
                "archived",
                serde_json::json!({"action": if archived { "archive" } else { "reopen" }}),
            )
        }
    };
    let origin = m.origin_node.to_string();
    let counter = if m.ingest_seq.is_some() {
        m.origin_counter
    } else {
        m.decision_seq
    };
    let key = format!("register:{thread}:{field}:{}:{counter}", &origin[..8]);
    let event_json = event_payload.to_string();
    let (event, _) = schema::append_event_once_with_decision_seq(
        tx,
        EventInput {
            thread: &payload.thread,
            key: &key,
            kind: "info",
            payload_json: &event_json,
            decision_at: m.decision.utc,
            source_message: None,
            source_invitation: None,
        },
        m.decision_seq,
    )?;
    match payload.field {
        RegisterField::Topic => {
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
                &payload.thread,
                u64::try_from(seq).map_err(|_| {
                    api_error(
                        ErrorCode::StoreCorrupt,
                        "negative context decision sequence",
                    )
                })?,
            )?;
            schema::bump_filter_revision(tx, &instance, "topic", thread)?;
            schema::bump_filter_revision(tx, &instance, "topic", "all")?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
        }
        RegisterField::Name => {
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            schema::bump_filter_revision(tx, &instance, "directory", "name/all")?;
        }
        RegisterField::Archived => {
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
        }
    }
    Ok(())
}

/// Materializes `thread.direct` (section 5): inserts the pair's thread and its
/// `direct_threads` row on first apply (the default topic is a pure function of
/// the pair, so it is not a register). The creator is a min-register: a later
/// apply of an earlier `(hlc, node)` updates only `created_hlc`,
/// `created_node` and `threads.created_at`, so every node converges on the
/// earliest creator whatever the apply order; it appends no event.
pub fn materialize_thread_direct(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &ThreadDirectPayload,
) -> Result<(), ApiError> {
    let hlc = i64::try_from(m.origin_hlc.as_u64())
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "hlc out of range"))?;
    let node = m.origin_node.to_string();
    let existing: Option<(i64, String)> = tx
        .query_row(
            "SELECT created_hlc,created_node FROM direct_threads WHERE thread_id=?1",
            [payload.thread.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    if let Some((created_hlc, created_node)) = existing {
        if (hlc, node.as_str()) < (created_hlc, created_node.as_str()) {
            tx.execute(
                "UPDATE direct_threads SET created_hlc=?1,created_node=?2 WHERE thread_id=?3",
                params![hlc, node, payload.thread.as_str()],
            )
            .map_err(store_error)?;
            tx.execute(
                "UPDATE threads SET created_at=?1 WHERE id=?2",
                params![m.origin_ms, payload.thread.as_str()],
            )
            .map_err(store_error)?;
        }
        return Ok(());
    }
    let instance = local_instance(tx)?;
    let label = format!(
        "1:1 {} {}",
        payload.seat_lo.as_str(),
        payload.seat_hi.as_str()
    );
    tx.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES(?1,?2,?3,?3,?4,?4,1)",
        params![payload.thread.as_str(), instance, label, m.origin_ms],
    )
    .map_err(store_error)?;
    tx.execute(
        "INSERT INTO direct_threads(instance_id,seat_a,seat_b,thread_id,created_hlc,created_node) VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            instance,
            payload.seat_lo.as_str(),
            payload.seat_hi.as_str(),
            payload.thread.as_str(),
            hlc,
            node
        ],
    )
    .map_err(store_error)?;
    let event = serde_json::json!({"action":"create_thread","goal":label}).to_string();
    create_event(tx, m, &payload.thread, &event)?;
    schema::bump_filter_revision(tx, &instance, "directory", "all")?;
    Ok(())
}

/// Authors one `thread.register` write for a command or the archival lane:
/// through the journal for a journaled thread (above the register's current
/// stamp, so it wins against what the author saw), as a node-local write for
/// any other. `ctx` is `None` only for a caller that holds no store context
/// (the bare archival entry); that is fine for a node-local thread and
/// `Unsupported` for a journaled one.
pub(crate) fn author_register(
    tx: &Transaction<'_>,
    ctx: Option<&StoreContext>,
    decision: DecisionInstant,
    payload: &ThreadRegisterPayload,
) -> Result<(), ApiError> {
    let route = FactRoute::for_thread(tx, &payload.thread)?;
    // Transitional (ht-5fr.4.10): a pair thread `ensure_direct_thread` still
    // inserts directly has no creating fact, so a register fact for it would
    // park on its `needs`. Until that command authors `thread.direct`, such a
    // thread keeps the node-local write it always had.
    let route = if route == FactRoute::Journal && !has_creating_fact(tx, &payload.thread)? {
        FactRoute::LocalOnly
    } else {
        route
    };
    match (route, ctx) {
        (FactRoute::Journal, Some(ctx)) => {
            let floor = thread_registers::current(tx, &payload.thread, store_field(payload.field))
                .map_err(store_error)?
                .map(|(hlc, _)| hlc);
            FactRoute::Journal.emit::<ThreadRegister>(
                tx,
                ctx,
                decision,
                payload,
                floor,
                materialize_register,
            )?;
        }
        (FactRoute::Journal, None) => {
            return Err(api_error(ErrorCode::Unsupported, "journal not open"));
        }
        (FactRoute::LocalOnly, _) => {
            let m = local_only(tx, decision)?;
            materialize_register(tx, &m, payload)?;
        }
    }
    Ok(())
}

/// Whether some journal entry created the thread's key (`thread.created` or
/// `thread.direct`).
fn has_creating_fact(tx: &Transaction<'_>, thread: &ThreadId) -> Result<bool, ApiError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM journal WHERE creates=?1)",
        [thread.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

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
            let current: bool = tx
                .query_row(
                    "SELECT archived FROM threads WHERE id=?1",
                    [command.thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if current != archived {
                author_register(
                    tx,
                    Some(context),
                    decision,
                    &ThreadRegisterPayload {
                        thread: command.thread.clone(),
                        field: RegisterField::Archived,
                        value: Some(if archived { "1" } else { "0" }.to_owned()),
                    },
                )?;
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
                author_register(
                    tx,
                    Some(context),
                    at,
                    &ThreadRegisterPayload {
                        thread: command.thread.clone(),
                        field: RegisterField::Topic,
                        value: Some(command.topic.clone()),
                    },
                )?;
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
                author_register(
                    tx,
                    Some(context),
                    at,
                    &ThreadRegisterPayload {
                        thread: command.thread.clone(),
                        field: RegisterField::Name,
                        value: command.name.clone(),
                    },
                )?;
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
            // The creation fact inserts the thread, seeds its registers and
            // appends the `create:` event; the creator's membership (still a
            // local write) joins at that event's position so it sees it.
            let created = ThreadCreatedPayload {
                thread: thread.clone(),
                topic: command.topic.clone(),
                goal: command.goal.clone(),
                name: command.name.clone(),
                creator: seat.clone(),
            };
            if context.journal_open() {
                FactRoute::Journal.emit::<ThreadCreated>(
                    tx,
                    context,
                    decision,
                    &created,
                    None,
                    |_, _, _| {
                        Err(api_error(
                            ErrorCode::Unsupported,
                            "thread.created is journaled",
                        ))
                    },
                )?;
            } else {
                // Only a store whose journal never opened (a fixture; the
                // daemon opens it before serving) gets here: the same
                // materializer, written locally.
                let input = local_only(tx, decision)?;
                materialize_thread_created(
                    tx,
                    &input,
                    &NewThread {
                        thread: &thread,
                        topic: &created.topic,
                        goal: &created.goal,
                        name: created.name.as_deref(),
                        creator: Some(&seat),
                        managed_owner: None,
                        journaled: true,
                    },
                )?;
            }
            let joined_seq: i64 = tx
                .query_row(
                    "SELECT decision_seq FROM messages WHERE event_key=?1",
                    [format!("create:{}", thread.as_str())],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
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
            tx.execute("INSERT INTO memberships(thread_id, seat_id, state, joined_at, home_transition, last_joined_n) VALUES (?1,?2,'joined',?3,1,1)",
                params![thread.as_str(), seat.as_str(), decision.utc.0]).map_err(store_error)?;
            tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,1,?3)",
                params![thread.as_str(),seat.as_str(),joined_seq]).map_err(store_error)?;
            crate::store::read_cursors::init_for_join(tx, &seat, &thread)?;
            schema::bump_membership_revision(tx, &thread)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &seat)?;
            schema::bump_filter_revision(tx, &instance, "inbox", seat.as_str())?;
            Ok(CommandResult::ThreadCreated(thread))
        },
    )
}
