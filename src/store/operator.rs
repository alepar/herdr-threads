//! Payload-bound historical replay and legacy operator digest tuples.
use crate::{
    protocol::{
        authority::OperatorActor,
        commands::{Command, OperatorCommand},
        ids::OperationId,
        results::{ApiError, CommandResult, ErrorCode},
        time::CallBudget,
    },
    store::{
        connection::{StoreContext, api_error},
        schema,
    },
};
use rusqlite::{OptionalExtension, params};

pub(crate) fn operation(command: &OperatorCommand) -> &OperationId {
    match command {
        OperatorCommand::Rebind(c) => &c.operation,
        OperatorCommand::FreshSeat(c) => &c.operation,
        OperatorCommand::OrphanInvite(c) => &c.operation,
        OperatorCommand::Retire(c) => &c.operation,
        OperatorCommand::Replace(c) => &c.operation,
    }
}
/// Preserve existing persisted tuples; changing these requires a migration.
pub(crate) fn digest(instance: &str, command: &OperatorCommand) -> Result<[u8; 32], ApiError> {
    if instance.is_empty() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "operator instance required",
        ));
    }
    let wire = match command {
        OperatorCommand::Rebind(c) => Command::OperatorRebind(c.clone()),
        OperatorCommand::FreshSeat(c) => Command::OperatorFreshSeat(c.clone()),
        OperatorCommand::OrphanInvite(c) => Command::OperatorOrphanInvite(c.clone()),
        OperatorCommand::Retire(c) => Command::OperatorRetire(c.clone()),
        OperatorCommand::Replace(c) => Command::OperatorReplace(c.clone()),
    };
    wire.validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    match command {
        OperatorCommand::Rebind(c) => {
            schema::canonical_digest(&("operator_rebind", instance, &c.target, Some(&c.seat)))
        }
        OperatorCommand::FreshSeat(c) => schema::canonical_digest(&(
            "operator_fresh",
            instance,
            &c.target,
            Option::<&crate::protocol::ids::SeatId>::None,
        )),
        OperatorCommand::Retire(c) => {
            schema::canonical_digest(&("operator_retire", instance, &c.seat))
        }
        OperatorCommand::Replace(c) => schema::canonical_digest(&(
            "operator_replace",
            instance,
            &c.target,
            &c.seat,
            &c.replace,
        )),
        OperatorCommand::OrphanInvite(c) => {
            if c.deadline_millis.is_some_and(|d| d > i64::MAX as u64) {
                return Err(api_error(
                    ErrorCode::InvalidRequest,
                    "invitation duration too large",
                ));
            }
            schema::canonical_digest(&(
                "operator_orphan_invite",
                &c.thread,
                &c.seat,
                c.deadline_millis,
            ))
        }
    }
}
pub(crate) fn validate_result(
    command: &OperatorCommand,
    result: &CommandResult,
) -> Result<(), ApiError> {
    let valid = match (command, result) {
        (OperatorCommand::Rebind(c), CommandResult::OperatorRebound(seat)) => seat == &c.seat,
        (OperatorCommand::Retire(c), CommandResult::OperatorRetired(seat)) => seat == &c.seat,
        (OperatorCommand::Replace(c), CommandResult::OperatorRebound(seat)) => seat == &c.seat,
        (OperatorCommand::FreshSeat(_), CommandResult::OperatorFreshSeat(_))
        | (OperatorCommand::OrphanInvite(_), CommandResult::OperatorInvited(_)) => true,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(api_error(
            ErrorCode::StoreCorrupt,
            "operator replay result has wrong type or source",
        ))
    }
}
pub(crate) fn replay(
    context: &StoreContext,
    instance: &str,
    command: &OperatorCommand,
    actor: &OperatorActor,
    budget: &CallBudget,
) -> Result<Option<CommandResult>, ApiError> {
    let expected = digest(instance, command)?;
    let db = context.open_query(budget.clone())?;
    // Opaque IDs are at most128 ASCII bytes; worst-case JSON escaping
    // doubles that size.512 bytes includes every legacy operator wrapper.
    let stored: Option<(Option<Vec<u8>>, Option<String>)> = db
        .query_row(
            "SELECT CASE WHEN typeof(digest)='blob' AND length(digest)=32 THEN digest END, CASE WHEN typeof(result_json)='text' AND length(CAST(result_json AS BLOB))<=512 THEN result_json END FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![actor.operation_scope(instance), operation(command).as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| db.map_error(e))?;
    let Some((digest, json)) = stored else {
        return Ok(None);
    };
    let digest = digest
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "invalid operator digest shape"))?;
    if digest != expected {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "operation key reused with different payload",
        ));
    }
    let json = json.ok_or_else(|| {
        api_error(
            ErrorCode::StoreCorrupt,
            "invalid operator result encoding or byte bound",
        )
    })?;
    let result = serde_json::from_str(&json).map_err(|e| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("invalid stored result: {e}"),
        )
    })?;
    validate_result(command, &result)?;
    Ok(Some(result))
}
#[cfg(test)]
#[path = "../../tests/store/operator.rs"]
mod tests;
