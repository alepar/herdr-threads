//! Nudge threshold and wake-on.

use crate::{
    protocol::{
        authority::{MutationPermit, ObligationRef},
        ids::SeatId,
        results::{ApiError, CommandResult, ErrorCode},
        time::CallBudget,
    },
    store::{
        connection::{StoreContext, api_error, store_error},
        control, schema,
    },
};
use rusqlite::{Connection, OptionalExtension, params};

use super::*;

/// The seat's current nudge threshold (D1, D2): read at every wake decision, never cached. A fresh
/// seat reads 0.
pub fn nudge_threshold(
    db: &Connection,
    seat: &SeatId,
) -> Result<crate::protocol::priority::NudgeThreshold, ApiError> {
    require_local_threshold_seat(db, seat)?;
    let stored: Option<i64> = db
        .query_row(
            "SELECT nudge_threshold FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let stored = stored.ok_or_else(|| api_error(ErrorCode::NotFound, "seat not found"))?;
    crate::protocol::priority::NudgeThreshold::from_stored(stored)
}

/// Row priority `r.priority` admitted by the seat's threshold, for set-based SQL. Aliases: `r` =
/// `lazy_recipients`, `s` = `seats`. The SQL twin of `NudgeThreshold::admits`; no statement writes
/// `p <= T` by hand. `threshold_admits_sql!` is the same text for `concat!`.
macro_rules! threshold_admits_sql {
    () => {
        "(s.nudge_threshold<0 OR r.priority<=s.nudge_threshold)"
    };
}
pub(crate) use threshold_admits_sql;
pub(crate) const THRESHOLD_ADMITS_SQL: &str = threshold_admits_sql!();

/// The seat's `wake_on` preference through the legacy alias: `all` iff the threshold is negative.
pub fn wake_on(
    db: &Connection,
    seat: &SeatId,
) -> Result<crate::protocol::commands::WakeOn, ApiError> {
    Ok(nudge_threshold(db, seat)?.legacy_wake_on())
}

/// Set the caller seat's `wake_on` (TRUST-POLICY A5) through the nudge threshold
/// (`all` -> -1, `nudge` -> 0). `changed` means the threshold changed.
pub fn set_wake_on(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &crate::protocol::commands::SetWakeOn,
    permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    let value = crate::protocol::priority::NudgeThreshold::from_wake_on(command.value);
    let result = write_threshold(
        context,
        conn,
        budget,
        permit,
        &command.claim,
        &command.operation,
        control::cooperative_payload_hash("set_wake_on", command)?,
        "declared subagents cannot set wake_on",
        value,
    )?;
    // A replay of a pre-threshold operation returns its stored `WakeOn` result as is.
    Ok(match result {
        CommandResult::NudgeThreshold(state) => {
            CommandResult::WakeOn(crate::protocol::results::WakeOnState {
                seat: state.seat,
                wake_on: command.value,
                changed: state.changed,
            })
        }
        other => other,
    })
}

/// Set the caller seat's nudge threshold (TRUST-POLICY A5): `set_wake_on`'s twin.
pub fn set_nudge_threshold(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &crate::protocol::commands::SetNudgeThreshold,
    permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    write_threshold(
        context,
        conn,
        budget,
        permit,
        &command.claim,
        &command.operation,
        control::cooperative_payload_hash("set_nudge_threshold", command)?,
        "declared subagents cannot set the nudge threshold",
        command.value,
    )
}

/// The accountable cooperative transaction behind both setters: scoped `seat:<id>`, decided against
/// the seat's current binding. Only a top-level claim may set it. A change writes the threshold
/// and the audit column `wake_on`, then re-judges the seat's wake reasons through
/// `wake::note_new_attention`; setting the stored value again records nothing. The preference is
/// wake eligibility only: it creates no receipt, ACK evidence or authority.
#[allow(clippy::too_many_arguments)]
fn write_threshold(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    mut permit: MutationPermit,
    claim: &crate::protocol::authority::CallerClaim,
    operation: &crate::protocol::ids::OperationId,
    digest: [u8; 32],
    subagent_refusal: &str,
    value: crate::protocol::priority::NudgeThreshold,
) -> Result<CommandResult, ApiError> {
    use crate::protocol::authority::CallerRole;
    if claim.role == CallerRole::Subagent {
        return Err(api_error(ErrorCode::InvalidRequest, subagent_refusal));
    }
    let seat = claim.seat.clone();
    // Before the transaction: its claim precheck would refuse a remote seat with the less useful
    // "cooperative seat is not local".
    require_local_threshold_seat(conn, &seat)?;
    let obligation = ObligationRef::CheckIn(seat.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        permit.cooperative_metadata(),
        &format!("seat:{}", seat.as_str()),
        operation.as_str(),
        digest,
        |_| Ok(()),
        |tx, at| {
            control::decide_accountable(
                context,
                tx,
                at,
                &mut permit,
                claim,
                &seat,
                operation,
                &obligation,
                &digest,
            )?;
            let previous = nudge_threshold(tx, &seat)?;
            let changed = previous != value;
            if changed {
                tx.execute(
                    "UPDATE seats SET nudge_threshold=?1,wake_on=?2 WHERE id=?3",
                    params![
                        i64::from(value),
                        value.legacy_wake_on().as_str(),
                        seat.as_str()
                    ],
                )
                .map_err(store_error)?;
                super::super::wake::note_new_attention(tx, &seat)?;
            }
            Ok(CommandResult::NudgeThreshold(
                crate::protocol::results::NudgeThresholdState {
                    seat: seat.clone(),
                    threshold: value,
                    changed,
                },
            ))
        },
    )
}

#[cfg(test)]
#[path = "../../../tests/store/wake_on.rs"]
mod wake_on_tests;
