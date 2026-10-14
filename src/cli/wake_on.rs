//! `wake-on [nudge|all]` (spec DP8): read or set the invoking seat's wake
//! preference. A bare `wake-on` reads; a value sets it as the seat's current
//! top-level binding (TRUST-POLICY A5). The daemon decides everything; this
//! only builds the claim and formats the answer.

use super::{
    RunError,
    commands::{CliAction, CooperativeSelection, ParsedCli},
};
use crate::daemon::paths::InstancePaths;
use crate::ports::LocalClient;
use crate::{
    harness::context::Role,
    protocol::{
        capabilities::{SEAT_NUDGE_THRESHOLD_V1, SEAT_WAKE_ON_V1},
        commands::{Command, SetWakeOn, WakeOn, WakeOnQuery},
        ids::OperationId,
        output::OutputFormat,
        priority::NudgeThreshold,
        results::{CommandResult, WakeOnState},
        time::Clock,
    },
};
use std::io::Write;

/// The text line: `wake_on: all (was nudge)`. The previous value is derived:
/// a change flipped the other of the two values; no change kept it.
pub fn render(state: &WakeOnState, set: bool) -> String {
    if !set {
        return format!("wake_on: {}\n", state.wake_on.as_str());
    }
    let was = if state.changed {
        match state.wake_on {
            WakeOn::Nudge => WakeOn::All,
            WakeOn::All => WakeOn::Nudge,
        }
    } else {
        state.wake_on
    };
    format!(
        "wake_on: {} (was {})\n",
        state.wake_on.as_str(),
        was.as_str()
    )
}

/// Run `wake-on` as the selected seat.
pub(crate) fn run<C: LocalClient + ?Sized, W: Write>(
    parsed: ParsedCli,
    selection: &CooperativeSelection,
    paths: &InstancePaths,
    instance: uuid::Uuid,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<(), RunError> {
    let CliAction::WakeOn(value) = &parsed.action else {
        unreachable!("wake_on::run is only called for wake-on actions");
    };
    let budget = super::cooperative_budget(clock);
    // Deprecated alias: against a daemon with the nudge threshold, `wake-on V` is `nudge-threshold`
    // of the mapped value, plus one stderr deprecation line (about 55 bytes).
    if client.supports_capability(SEAT_NUDGE_THRESHOLD_V1, &budget) {
        super::nudge_threshold::run(
            &parsed,
            value.map(NudgeThreshold::from_wake_on),
            selection,
            paths,
            instance,
            client,
            clock,
            writer,
        )?;
        eprintln!("herdr-threads: wake-on is deprecated; use nudge-threshold N");
        return Ok(());
    }
    if !client.supports_capability(SEAT_WAKE_ON_V1, &budget) {
        return Err(super::unsupported(
            "this daemon does not support wake-on; restart it on the current herdr-threads",
        ));
    }
    let command = match value {
        None => Command::WakeOn(WakeOnQuery {
            seat: selection.seat.clone(),
        }),
        Some(value) => {
            if selection.role != Role::TopLevel {
                return Err(super::unsupported(
                    "subagents cannot set wake-on; only the top-level agent does",
                ));
            }
            let claim = current_claim(selection, paths, instance, client, &parsed, clock)?;
            Command::SetWakeOn(SetWakeOn {
                value: *value,
                operation: OperationId::new(uuid::Uuid::new_v4().to_string()),
                claim,
            })
        }
    };
    command.validate().map_err(super::invalid_request)?;
    let result = client.call_with_output(command, &parsed.output, &budget)?;
    let CommandResult::WakeOn(state) = &result else {
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

/// The seat's saved context as a claim, after proving it is the service's
/// current binding generation (the same check `summary` makes).
pub(super) fn current_claim<C: LocalClient + ?Sized>(
    selection: &CooperativeSelection,
    paths: &InstancePaths,
    instance: uuid::Uuid,
    client: &C,
    parsed: &ParsedCli,
    clock: &dyn Clock,
) -> Result<crate::protocol::authority::CallerClaim, RunError> {
    use crate::protocol::{commands::SeatInspectQuery, pagination::PageRequest};
    let inspection = client.call_with_output(
        Command::SeatInspect(SeatInspectQuery {
            seat: selection.seat.clone(),
            page: PageRequest {
                cursor: None,
                limit: 1,
                max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
            },
        }),
        &parsed.output,
        &super::cooperative_budget(clock),
    )?;
    let CommandResult::SeatInspect(inspection) = inspection else {
        return Err(super::mapping_error("service returned no seat inspection"));
    };
    let generation = super::selected_generation(selection, &inspection)?;
    // An agent caller acts as its harness session (decision 15): its context is derived from the
    // environment at the seat's mapping generation, never the saved pane context.
    let contexts = super::caller_journal(
        super::seat_contexts(paths, instance, &selection.seat)?,
        instance,
        selection,
        generation,
    )?;
    let context = contexts
        .current()
        .map_err(super::context_run_error)?
        .ok_or_else(|| {
            super::caller_not_located("context missing; explicit lifecycle check-in required")
        })?;
    if context.instance != instance
        || context.seat != selection.seat.as_str()
        || context.target != selection.target.as_str()
        || context.harness != selection.harness
        || context.binding_generation != generation
    {
        return Err(super::mapping_error(
            "local context differs from current service mapping",
        ));
    }
    contexts.claim(&context).map_err(super::context_run_error)
}

#[cfg(test)]
#[path = "../../tests/cli/wake_on.rs"]
mod tests;
