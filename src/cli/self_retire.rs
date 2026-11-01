//! `seat retire --self [--force]` (SA6, D5): the invoking named seat retires itself.
//! The daemon decides everything (M20 for a seat without a name, M18 while another top-level
//! binding is connected, replay by operation); this builds the claim and prints the answer.

use super::{
    RunError,
    commands::{CooperativeSelection, ParsedCli},
    site_capability,
};
use crate::daemon::paths::NodePaths;
use crate::ports::LocalClient;
use crate::{
    harness::context::Role,
    protocol::{
        capabilities::HOST_SITES_V1,
        commands::{Command, SelfRetire},
        ids::OperationId,
        output::OutputFormat,
        results::{AbandonedKind, ApiError, CommandResult, ErrorCode, SelfRetired},
        time::Clock,
    },
};
use std::io::Write;

/// The text answer: one `ack <message-id>` or `invite <invitation-id>` row per abandoned item
/// (about 30 bytes each), a `+N more` row when the list was cut, then `retired <seat>`.
pub fn render(retired: &SelfRetired) -> String {
    let mut out = String::new();
    for item in &retired.abandoned {
        let kind = match item.kind {
            AbandonedKind::Invitation => "invite",
            AbandonedKind::Ack => "ack",
        };
        out.push_str(&format!("{kind} {}\n", item.id));
    }
    if retired.more > 0 {
        out.push_str(&format!("+{} more\n", retired.more));
    }
    out.push_str(&format!("retired {}\n", retired.seat.as_str()));
    out
}

/// A refusal whose detail is already the full `herdr-threads: ...` line (M13, M18, M20) is
/// printed as that line alone, keeping the error's exit status.
fn as_line(error: RunError) -> RunError {
    if let RunError::Api(api) = &error
        && api.detail.starts_with("herdr-threads: ")
    {
        eprintln!("{}", api.detail.trim_end());
        return RunError::Exit(error.exit_code());
    }
    error
}

/// Run `seat retire --self` as the selected seat.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run<C: LocalClient + ?Sized, W: Write>(
    parsed: &ParsedCli,
    force: bool,
    selection: &CooperativeSelection,
    paths: &NodePaths,
    instance: crate::node::NodeUuid,
    host: &crate::protocol::ids::HostLocator,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<(), RunError> {
    let budget = super::cooperative_budget(clock);
    if selection.role != Role::TopLevel {
        return Err(super::unsupported(
            "subagents cannot retire a seat; only the top-level agent does",
        ));
    }
    // A daemon without the named-site capability predates the command: the existing upgrade line.
    if !client.supports_capability(HOST_SITES_V1, &budget) {
        return Err(as_line(RunError::Api(ApiError::new(
            ErrorCode::Unsupported,
            site_capability::M13,
        ))));
    }
    let claim =
        super::wake_on::current_claim(selection, paths, instance, host, client, parsed, clock)?;
    let command = Command::SelfRetire(SelfRetire {
        operation: OperationId::new(uuid::Uuid::new_v4().to_string()),
        claim,
        force,
    });
    command.validate().map_err(super::invalid_request)?;
    let result = client
        .call_with_output(command, &parsed.output, &budget)
        .map_err(|error| as_line(RunError::Api(error)))?;
    let CommandResult::SelfRetired(retired) = &result else {
        return Err(super::mapping_error(
            "service returned an unexpected result",
        ));
    };
    let bytes = if parsed.output.format == OutputFormat::Json {
        crate::protocol::output::encode_selected(&result, &parsed.output)?
    } else {
        render(retired).into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ids::SeatId, results::AbandonedItem};

    // Kills: a dropped `+N more` row, a swapped kind word, or `retired` not last.
    #[test]
    fn render_lists_items_the_cut_and_the_seat_last() {
        let retired = SelfRetired {
            seat: SeatId::new("seat-Xy98Zw76"),
            abandoned: vec![
                AbandonedItem {
                    kind: AbandonedKind::Invitation,
                    id: "inv-1".into(),
                },
                AbandonedItem {
                    kind: AbandonedKind::Ack,
                    id: "mnSGraaKyKMx@dBq9KyMdCb6".into(),
                },
            ],
            more: 7,
        };
        assert_eq!(
            render(&retired),
            "invite inv-1\nack mnSGraaKyKMx@dBq9KyMdCb6\n+7 more\nretired seat-Xy98Zw76\n"
        );
        let none = SelfRetired {
            abandoned: vec![],
            more: 0,
            ..retired
        };
        assert_eq!(render(&none), "retired seat-Xy98Zw76\n");
    }
}
