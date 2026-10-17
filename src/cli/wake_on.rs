//! `wake-on [nudge|all]`: the deprecated alias of `nudge-threshold` (`all` = -1, `nudge` = 0), plus
//! one stderr deprecation line.

use super::{
    RunError,
    commands::{CliAction, CooperativeSelection, ParsedCli},
};
use crate::daemon::paths::NodePaths;
use crate::ports::LocalClient;
use crate::protocol::{
    commands::Command, priority::NudgeThreshold, results::CommandResult, time::Clock,
};
use std::io::Write;

/// Run `wake-on` as the selected seat: `wake-on V` is `nudge-threshold` of the mapped value, plus
/// one stderr deprecation line (about 55 bytes).
// Allowed: the parsed action and selected seat, node paths and instance, claim host, client, clock and writer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run<C: LocalClient + ?Sized, W: Write>(
    parsed: ParsedCli,
    selection: &CooperativeSelection,
    paths: &NodePaths,
    instance: crate::node::NodeUuid,
    host: &crate::protocol::ids::HostLocator,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<(), RunError> {
    let CliAction::WakeOn(value) = &parsed.action else {
        unreachable!("wake_on::run is only called for wake-on actions");
    };
    super::nudge_threshold::run(
        &parsed,
        value.map(NudgeThreshold::from_wake_on),
        selection,
        paths,
        instance,
        host,
        client,
        clock,
        writer,
    )?;
    eprintln!("herdr-threads: wake-on is deprecated; use nudge-threshold N");
    Ok(())
}

/// The seat's saved context as a claim, after proving it is the service's
/// current binding generation (the same check `summary` makes).
pub(super) fn current_claim<C: LocalClient + ?Sized>(
    selection: &CooperativeSelection,
    paths: &NodePaths,
    instance: crate::node::NodeUuid,
    host: &crate::protocol::ids::HostLocator,
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
        super::seat_contexts(paths, instance, host, &selection.seat)?,
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
