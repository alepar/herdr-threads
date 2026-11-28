//! `config seat list|get|set|unset` and the person's `human config seat set|unset` (ht-z41e):
//! per-seat settings, rendered from the closed registry (`protocol::seat_config`).
//!
//! `list` is static: it reads the registry and never contacts the daemon. The other verbs gate on
//! `seat.config_v1`, then send `GetSeatConfig` / `SetSeatConfig` / `UnsetSeatConfig`. The tier is
//! the invocation namespace: only an immediate `human` argv[1] sends `tier: human` (no claim, an
//! explicit seat); every other caller is the agent tier on its own seat through its claim. The
//! daemon decides every write; this builds the request and formats the answer.

use super::{
    RunError,
    commands::{CliAction, ConfigSeatAction, CooperativeSelection, ParsedCli},
    site_capability::require_site_capability,
};
use crate::daemon::paths::NodePaths;
use crate::harness::context::Role;
use crate::ports::LocalClient;
use crate::protocol::{
    capabilities::SEAT_CONFIG_V1,
    commands::{Command, GetSeatConfig, SetSeatConfig, UnsetSeatConfig},
    ids::OperationId,
    output::OutputFormat,
    results::{CommandResult, SeatConfigEntry, SeatConfigWrite},
    seat_config::{ConfigTier, REGISTRY},
    time::Clock,
};
use std::io::Write;

/// The daemon does not advertise `seat.config_v1` (54 bytes).
pub const SEAT_CONFIG_REFUSAL: &str = "herdr-threads: daemon predates seat config; restart it";

/// `config seat get` with no `--seat` and no caller seat (62 bytes).
pub const GET_NEEDS_SEAT: &str = "herdr-threads: config seat get needs --seat S (no caller seat)";

/// `human config seat set|unset` with no `--seat` and no caller seat (64 bytes).
pub const HUMAN_NEEDS_SEAT: &str =
    "herdr-threads: human config seat needs --seat S (no caller seat)";

/// A subagent writing seat config (63 bytes).
pub const SUBAGENT_REFUSAL: &str =
    "subagents cannot set seat config; only the top-level agent does";

/// The last line of `list`, so an agent shown only that output can name the human spelling.
pub const LIST_TRAILER: &str = "set: ht config seat set KEY VALUE (agent tier); ht human config seat set [--seat S] KEY VALUE (any key)";

/// One `list` line per registry key (under 160 bytes each).
pub fn render_list() -> String {
    let mut out = String::new();
    for key in REGISTRY {
        out.push_str(&format!(
            "{} {} default={} tier={} \u{2014} {}\n",
            key.name,
            key.values_label(),
            key.default,
            key.tier.as_str(),
            key.description
        ));
    }
    out.push_str(LIST_TRAILER);
    out.push('\n');
    out
}

/// `list --json`: an array of `{key, values, default, tier, description}`.
pub fn render_list_json() -> Result<String, serde_json::Error> {
    let rows: Vec<_> = REGISTRY
        .iter()
        .map(|key| {
            serde_json::json!({
                "key": key.name,
                "values": key.values_label(),
                "default": key.default,
                "tier": key.tier.as_str(),
                "description": key.description,
            })
        })
        .collect();
    Ok(serde_json::to_string(&rows)? + "\n")
}

/// `list`: static, no daemon, no caller.
pub(crate) fn run_list<W: Write>(parsed: &ParsedCli, writer: &mut W) -> Result<(), RunError> {
    let text = if parsed.output.format == OutputFormat::Json {
        render_list_json().map_err(|error| super::mapping_error(&error.to_string()))?
    } else {
        render_list()
    };
    writer.write_all(text.as_bytes())?;
    writer.flush()?;
    Ok(())
}

/// One `get` row: `<key>: <value>`, plus ` (default)` when the key is unset.
fn render_entry(entry: &SeatConfigEntry) -> String {
    let note = if entry.is_default { " (default)" } else { "" };
    format!("{}: {}{note}\n", entry.key, entry.value)
}

/// A write's line: `<key>: <value> (changed|unchanged)`.
fn render_write(write: &SeatConfigWrite) -> String {
    let note = if write.changed {
        "changed"
    } else {
        "unchanged"
    };
    format!("{}: {} ({note})\n", write.key, write.value)
}

/// Run `get`, `set` or `unset`. `selection` is the caller's seat when one was derived (an agent
/// write needs it; a read or a human write uses it only as the default seat).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run<C: LocalClient + ?Sized, W: Write>(
    parsed: &ParsedCli,
    selection: Option<&CooperativeSelection>,
    paths: &NodePaths,
    instance: crate::node::NodeUuid,
    host: &crate::protocol::ids::HostLocator,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<(), RunError> {
    let CliAction::ConfigSeat(action) = &parsed.action else {
        unreachable!("config_seat::run is only called for config seat actions");
    };
    let budget = super::cooperative_budget(clock);
    let human = parsed.actor == super::actor_route::InvocationActor::Human;
    let caller_seat = || selection.map(|selection| selection.seat.clone());
    let command = match action {
        ConfigSeatAction::List => return run_list(parsed, writer),
        ConfigSeatAction::Get { key, seat } => {
            let seat = seat
                .clone()
                .or_else(caller_seat)
                .ok_or_else(|| super::invalid_request(GET_NEEDS_SEAT))?;
            gate(client, &budget)?;
            Command::GetSeatConfig(GetSeatConfig {
                seat,
                key: key.clone(),
            })
        }
        ConfigSeatAction::Set { .. } | ConfigSeatAction::Unset { .. } => {
            let (seat, claim) = if human {
                let seat = action
                    .seat()
                    .cloned()
                    .or_else(caller_seat)
                    .ok_or_else(|| super::invalid_request(HUMAN_NEEDS_SEAT))?;
                gate(client, &budget)?;
                (Some(seat), None)
            } else {
                let selection = selection.ok_or_else(|| {
                    super::caller_not_located("seat config writes need the caller's own seat")
                })?;
                if selection.role != Role::TopLevel {
                    return Err(super::unsupported(SUBAGENT_REFUSAL));
                }
                gate(client, &budget)?;
                let claim = super::wake_on::current_claim(
                    selection, paths, instance, host, client, parsed, clock,
                )?;
                (None, Some(claim))
            };
            let tier = if human {
                ConfigTier::Human
            } else {
                ConfigTier::Agent
            };
            let operation = OperationId::new(uuid::Uuid::new_v4().to_string());
            match action {
                ConfigSeatAction::Set { key, value, .. } => Command::SetSeatConfig(SetSeatConfig {
                    seat,
                    key: key.clone(),
                    value: value.clone(),
                    tier,
                    operation,
                    claim,
                }),
                ConfigSeatAction::Unset { key, .. } => Command::UnsetSeatConfig(UnsetSeatConfig {
                    seat,
                    key: key.clone(),
                    tier,
                    operation,
                    claim,
                }),
                _ => unreachable!("matched a write above"),
            }
        }
    };
    command.validate().map_err(super::invalid_request)?;
    let result = client.call_with_output(command, &parsed.output, &budget)?;
    let bytes = if parsed.output.format == OutputFormat::Json {
        crate::protocol::output::encode_selected(&result, &parsed.output)?
    } else {
        let text = match &result {
            CommandResult::SeatConfig(view) => view.entries.iter().map(render_entry).collect(),
            CommandResult::SeatConfigWritten(write) => {
                let mut text = render_write(write);
                if write.key == "nudge_threshold"
                    && super::nudge_threshold::has_no_idle_wake(client, &write.seat, &budget)
                {
                    text.push_str(&super::nudge_threshold::render_inert(&write.seat));
                }
                text
            }
            _ => {
                return Err(super::mapping_error(
                    "service returned an unexpected result",
                ));
            }
        };
        text.into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

/// Refuse before anything is sent when the daemon lacks `seat.config_v1`.
fn gate<C: LocalClient + ?Sized>(
    client: &C,
    budget: &crate::protocol::time::CallBudget,
) -> Result<(), RunError> {
    require_site_capability(client, SEAT_CONFIG_V1, SEAT_CONFIG_REFUSAL, budget)
        .map_err(RunError::Api)
}

#[cfg(test)]
#[path = "../../tests/cli/config_seat.rs"]
mod tests;
