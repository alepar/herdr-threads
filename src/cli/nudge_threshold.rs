//! `nudge-threshold [N]` (message-priority spec D5): read or set the invoking seat's nudge threshold.
//! A bare call reads; a value sets it as the seat's current top-level binding (TRUST-POLICY A5).
//! The daemon decides everything; this builds the claim and formats the answer.

use super::{
    RunError,
    commands::{CooperativeSelection, ParsedCli},
};
use crate::daemon::paths::NodePaths;
use crate::ports::LocalClient;
use crate::{
    harness::context::Role,
    protocol::{
        commands::{Command, NudgeThresholdQuery, SetNudgeThreshold},
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
    paths: &NodePaths,
    instance: crate::node::NodeUuid,
    host: &crate::protocol::ids::HostLocator,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<(), RunError> {
    let budget = super::cooperative_budget(clock);
    if value.is_some() && selection.role != Role::TopLevel {
        return Err(super::unsupported(
            "subagents cannot set nudge-threshold; only the top-level agent does",
        ));
    }
    let claim = match value {
        Some(_) => Some(super::wake_on::current_claim(
            selection, paths, instance, host, client, parsed, clock,
        )?),
        None => None,
    };
    let operation = || OperationId::new(uuid::Uuid::new_v4().to_string());
    let command = match (value, claim) {
        (None, _) => Command::NudgeThreshold(NudgeThresholdQuery {
            seat: selection.seat.clone(),
        }),
        (Some(value), Some(claim)) => Command::SetNudgeThreshold(SetNudgeThreshold {
            value,
            operation: operation(),
            claim,
        }),
        (Some(_), None) => unreachable!("a set always derives a claim"),
    };
    command.validate().map_err(super::invalid_request)?;
    let result = client.call_with_output(command, &parsed.output, &budget)?;
    let CommandResult::NudgeThreshold(state) = &result else {
        return Err(super::mapping_error(
            "service returned an unexpected result",
        ));
    };
    let bytes = if parsed.output.format == OutputFormat::Json {
        crate::protocol::output::encode_selected(&result, &parsed.output)?
    } else {
        render(state, value.is_some()).into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/cli/nudge_threshold.rs"]
mod tests;
