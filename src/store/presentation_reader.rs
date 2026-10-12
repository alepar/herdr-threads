//! Coalesced private presentation invalidation for an owning canonical reader operation.
//! This is passive maintenance, never presentation admission, receipt or wake authority.
use super::connection::{api_error, store_error};
use crate::protocol::{
    delivery::PresentationConsumer,
    ids::BindingId,
    results::{ApiError, ErrorCode},
};
use rusqlite::{Connection, OptionalExtension, params, types::Value};

struct BindingScope {
    seat: String,
    instance: String,
    generation: i64,
    execution: String,
    // Consumer rows have no second native-session field: the exact canonical BindingId
    // owns this session. Never select or substitute another binding by seat or generation.
    _native_session: String,
}

struct ConsumerScope {
    seat: String,
    instance: String,
    generation: i64,
    execution: String,
    mod_generation: i64,
    mod_execution: String,
    passive: Value,
    version: Value,
}

fn private_error(error: rusqlite::Error) -> ApiError {
    match error {
        rusqlite::Error::InvalidColumnType(..) => {
            api_error(ErrorCode::StoreCorrupt, "invalid reader presentation scope")
        }
        other => store_error(other),
    }
}

fn increment(value: Value, exhausted: &str) -> Result<i64, ApiError> {
    let Value::Integer(value) = value else {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "invalid reader presentation counter",
        ));
    };
    if value < 0 {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "negative reader presentation counter",
        ));
    }
    value
        .checked_add(1)
        .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, exhausted))
}

/// Call once per actually changed binding at the owning operation boundary, inside its existing
/// transaction. One binding/seat join and two fixed consumer point reads add at most two updates.
/// The join touches two indexed canonical relations; no binding history or consumer scan runs.
/// Errors propagate so the owner rolls back its cursors and every tentative private update.
pub(super) fn note_binding_reader_change(
    db: &Connection,
    binding: &BindingId,
) -> Result<(), ApiError> {
    let canonical = db
        .query_row(
            "SELECT b.seat_id,s.instance_id,b.generation,b.execution_id,b.native_session \
             FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id WHERE b.binding_id=?1",
            [binding.as_str()],
            |row| {
                Ok(BindingScope {
                    seat: row.get(0)?,
                    instance: row.get(1)?,
                    generation: row.get(2)?,
                    execution: row.get(3)?,
                    _native_session: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(private_error)?;
    let Some(canonical) = canonical else {
        return Ok(());
    };
    if canonical.generation < 0 {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "negative reader binding generation",
        ));
    }
    for consumer in [PresentationConsumer::Hook, PresentationConsumer::Watch] {
        let state = db
            .query_row(
                "SELECT seat_id,instance_id,generation,execution,mod_generation,mod_execution,\
                        ordinary_passive_generation,mod_version \
                 FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer=?2",
                params![binding.as_str(), consumer.as_str()],
                |row| {
                    Ok(ConsumerScope {
                        seat: row.get(0)?,
                        instance: row.get(1)?,
                        generation: row.get(2)?,
                        execution: row.get(3)?,
                        mod_generation: row.get(4)?,
                        mod_execution: row.get(5)?,
                        passive: row.get(6)?,
                        version: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(private_error)?;
        let Some(state) = state else {
            continue;
        };
        if state.seat != canonical.seat
            || state.instance != canonical.instance
            || !((state.generation == canonical.generation
                && state.execution == canonical.execution)
                || (state.mod_generation == canonical.generation
                    && state.mod_execution == canonical.execution))
        {
            continue;
        }
        if state.generation < 0 || state.mod_generation < 0 {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "negative reader presentation generation",
            ));
        }
        let passive = increment(state.passive, "ordinary passive generation exhausted")?;
        let version = increment(state.version, "passive content version exhausted")?;
        let changed = db
            .execute(
                "UPDATE invitation_presentation_consumers \
                 SET ordinary_passive_generation=?1,mod_version=?2 \
                 WHERE binding_id=?3 AND consumer=?4 AND seat_id=?5 AND instance_id=?6 \
                   AND ((generation=?7 AND execution=?8) OR (mod_generation=?7 AND mod_execution=?8)) \
                   AND ordinary_passive_generation=?9 AND mod_version=?10",
                params![
                    passive,
                    version,
                    binding.as_str(),
                    consumer.as_str(),
                    canonical.seat,
                    canonical.instance,
                    canonical.generation,
                    canonical.execution,
                    passive - 1,
                    version - 1,
                ],
            )
            .map_err(store_error)?;
        if changed != 1 {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "reader presentation scope changed",
            ));
        }
    }
    Ok(())
}
