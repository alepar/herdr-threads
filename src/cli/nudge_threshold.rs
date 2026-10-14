//! `nudge-threshold [N]` (message-priority spec D5): read or set the invoking seat's nudge threshold.
//! A bare call reads; a value sets it as the seat's current top-level binding (TRUST-POLICY A5).
//! The daemon decides everything; this builds the claim, formats the answer and, against a daemon
//! without `seat.nudge_threshold_v1`, maps -1 / 0 onto the older `wake-on` pair.

use super::{
    RunError,
    commands::{CooperativeSelection, ParsedCli},
};
use crate::daemon::paths::InstancePaths;
use crate::ports::LocalClient;
use crate::{
    harness::context::Role,
    protocol::{
        capabilities::{SEAT_NUDGE_THRESHOLD_V1, SEAT_WAKE_ON_V1},
        commands::{Command, NudgeThresholdQuery, SetNudgeThreshold, SetWakeOn, WakeOnQuery},
        ids::OperationId,
        output::OutputFormat,
        priority::NudgeThreshold,
        results::{CommandResult, NudgeThresholdState},
        time::Clock,
    },
};
use std::io::Write;

/// The text line: `nudge_threshold: 3` for a read, `nudge_threshold: 3 (changed)` or `(unchanged)`
/// for a set (at most 32 bytes).
pub fn render(state: &NudgeThresholdState, set: bool) -> String {
    let value = state.threshold.get();
    if !set {
        return format!("nudge_threshold: {value}\n");
    }
    let note = if state.changed {
        "changed"
    } else {
        "unchanged"
    };
    format!("nudge_threshold: {value} ({note})\n")
}

/// Run `nudge-threshold` (or its `wake-on` alias, which passes the mapped `value`) as the selected seat.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run<C: LocalClient + ?Sized, W: Write>(
    parsed: &ParsedCli,
    value: Option<NudgeThreshold>,
    selection: &CooperativeSelection,
    paths: &InstancePaths,
    instance: uuid::Uuid,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<(), RunError> {
    let budget = super::cooperative_budget(clock);
    let modern = client.supports_capability(SEAT_NUDGE_THRESHOLD_V1, &budget);
    if !modern {
        if !client.supports_capability(SEAT_WAKE_ON_V1, &budget) {
            return Err(super::unsupported(
                "this daemon does not support nudge-threshold; restart it on the current herdr-threads",
            ));
        }
        // Refused before any call: an older daemon can express only -1 and 0.
        if value.is_some_and(|v| v.to_wake_on().is_none()) {
            return Err(super::unsupported(
                "this daemon supports nudge-threshold -1 or 0 only; restart it on the current herdr-threads",
            ));
        }
    }
    if value.is_some() && selection.role != Role::TopLevel {
        return Err(super::unsupported(
            "subagents cannot set nudge-threshold; only the top-level agent does",
        ));
    }
    let claim = match value {
        Some(_) => Some(super::wake_on::current_claim(
            selection, paths, instance, client, parsed, clock,
        )?),
        None => None,
    };
    let operation = || OperationId::new(uuid::Uuid::new_v4().to_string());
    let command = match (modern, value, claim) {
        (true, None, _) => Command::NudgeThreshold(NudgeThresholdQuery {
            seat: selection.seat.clone(),
        }),
        (true, Some(value), Some(claim)) => Command::SetNudgeThreshold(SetNudgeThreshold {
            value,
            operation: operation(),
            claim,
        }),
        (false, None, _) => Command::WakeOn(WakeOnQuery {
            seat: selection.seat.clone(),
        }),
        (false, Some(value), Some(claim)) => Command::SetWakeOn(SetWakeOn {
            value: value.to_wake_on().expect("checked above"),
            operation: operation(),
            claim,
        }),
        (_, Some(_), None) => unreachable!("a set always derives a claim"),
    };
    command.validate().map_err(super::invalid_request)?;
    let result = client.call_with_output(command, &parsed.output, &budget)?;
    let state = match &result {
        CommandResult::NudgeThreshold(state) if modern => state.clone(),
        CommandResult::WakeOn(state) if !modern => NudgeThresholdState {
            seat: state.seat.clone(),
            threshold: NudgeThreshold::from_wake_on(state.wake_on),
            changed: state.changed,
        },
        _ => {
            return Err(super::mapping_error(
                "service returned an unexpected result",
            ));
        }
    };
    let bytes = if parsed.output.format == OutputFormat::Json {
        crate::protocol::output::encode_selected(&result, &parsed.output)?
    } else {
        render(&state, value.is_some()).into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/cli/nudge_threshold.rs"]
mod tests;
