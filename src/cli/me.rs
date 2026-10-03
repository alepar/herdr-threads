//! `herdr-threads me init`: a person's own seat identity for their pane.
//!
//! A person typing in a Herdr shell pane is a first-class occupant, not an
//! agent and not the administrative `--operator` repair actor. `me init`
//! resolves the invoking pane (`HERDR_PANE_ID`) to its seat by ordinary
//! resolution and runs the ordinary cooperative lifecycle CheckIn with
//! harness `human`, role top-level and a tagged plugin-context session. The
//! service records that binding, its availability and every later accountable
//! decision (send, invite, accept, ACK) with `operator_human` provenance, never
//! the agents' `cooperative_top_level` claim. The private per-seat context it
//! leaves behind is what lets later commands in the same pane locate the seat
//! with no caller flags, exactly as an agent's hook context does.
//!
//! It never takes over an agent: a pane where Herdr reports an agent, or whose
//! seat holds an agent's local context, is refused. Wake prompts are never
//! sent to a human seat; its mail waits to be read and ACKed by hand.

use super::{
    RunError, agent_evidence, agent_evidence_refusal,
    commands::{CliAction, CooperativeSelection, MutationSpec, ParsedCli},
    connect, invalid_request, journal, mapping_error, pane_agent, pane_seat, parse_pane, retry,
    run_selected, seat_contexts, selected_generation,
};
use crate::{
    daemon::paths::{InstancePaths, RuntimeContext},
    harness::context::{CheckInMode, Harness, Role},
    protocol::{
        commands::{Command, SeatInspectQuery},
        ids::{HostTargetId, SeatId},
        output::OutputSpec,
        pagination::PageRequest,
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};
use std::{io::Write, sync::Arc};

pub const ME_INIT_HELP: &str = "Run it in your own shell pane, then use thread create, invite, send, \
read, ack and accept there with no --cooperative-* flags. Your actions are recorded as \
operator_human, never as an agent. Messages remain readable, but a human seat owes no ACK and \
gets no overdue-ACK warning, even when a sender uses --require-ack. Re-run `me init` after a daemon restart to \
mark yourself available again. It is refused where agent markers (CLAUDECODE, CODEX_SANDBOX, \
CODEX_SANDBOX_NETWORK_DISABLED) or a Claude or Codex agent reported by Herdr are present, and over \
a seat bound to an agent; `me init --operator` overrides that as the local account (later \
commands in this pane then run as you).";

fn budget(clock: &dyn Clock, millis: u64) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(millis)),
        cancellation: Cancellation::default(),
    }
}

/// Ordinary resolution of the pane's seat: the existing mapping, or a seat
/// allocated with service allocation provenance (never an operator act).
fn resolve_seat(
    client: &crate::client::local::LocalSocketClient,
    paths: &InstancePaths,
    instance: uuid::Uuid,
    pane: &HostTargetId,
    clock: &dyn Clock,
) -> Result<SeatId, RunError> {
    let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
    let output = OutputSpec::default();
    let (_, result) = retry::run_new_api_to_writer_discarding_rejection(
        &journal,
        journal::IntentScope::ServiceAllocation {
            instance: instance.to_string(),
            target: pane.clone(),
        },
        journal::SemanticMutation::ResolveSeat {
            target: pane.clone(),
        },
        clock.utc_now().0,
        || {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "ordinary resolution has no caller claim",
            ))
        },
        |command| client.call_with_output_definitive(command, &output, &budget(clock, 5_000)),
        &output,
        &mut std::io::sink(),
    )
    .map_err(super::retry_failure)?;
    match result {
        CommandResult::SeatResolved(seat) => Ok(seat),
        _ => Err(mapping_error("service returned no resolved seat")),
    }
}

fn agent_seat_refusal(seat: &SeatId, pane: &HostTargetId, harness: Harness) -> RunError {
    invalid_request(&format!(
        "seat {seat} on pane {pane} belongs to a {harness:?} agent; `me init` never takes over an \
         agent's seat. Run it in your own shell pane, give this pane a fresh seat with \
         `herdr-threads seat resolve --pane {pane} --new-seat --operator`, or override as the \
         local account: `herdr-threads me init --operator`",
        seat = seat.as_str(),
        pane = pane.as_str(),
    ))
}

/// The CheckIn `me init` sends: replay an interrupted human lifecycle event,
/// continue a current human context at the service's generation, or start a
/// fresh lifecycle (first run, or after the service moved the seat on).
fn check_in_spec(
    contexts: &crate::harness::context::ContextJournal,
    seat: &SeatId,
    pane: &HostTargetId,
    generation: u64,
    operator: bool,
) -> Result<MutationSpec, RunError> {
    let context_error = super::context_run_error;
    let fresh = || MutationSpec::CheckInLifecycle {
        event_id: format!("me-init:{}", uuid::Uuid::new_v4()),
        native_session: None,
        operator,
    };
    if operator {
        // The override always starts a fresh lifecycle: a pending event from
        // an earlier, refused run would replay without the override.
        return Ok(fresh());
    }
    if let Some(pending) = contexts.pending().map_err(context_error)? {
        if pending.context.harness != Harness::Human {
            return Err(agent_seat_refusal(seat, pane, pending.context.harness));
        }
        return Ok(match pending.mode {
            CheckInMode::Lifecycle => MutationSpec::CheckInLifecycle {
                event_id: pending.event_id,
                native_session: None,
                operator: false,
            },
            CheckInMode::Current => MutationSpec::CheckIn,
        });
    }
    match contexts.current().map_err(context_error)? {
        Some(current) if current.harness != Harness::Human => {
            Err(agent_seat_refusal(seat, pane, current.harness))
        }
        Some(current) if current.binding_generation == generation => Ok(MutationSpec::CheckIn),
        _ => Ok(fresh()),
    }
}

/// Abandon this person's own pending lifecycle event (never an agent's).
fn discard_pending_human_event(
    journal: &journal::Journal,
    contexts: &crate::harness::context::ContextJournal,
) -> Result<(), RunError> {
    let context_error = super::context_run_error;
    let Some(pending) = contexts.pending().map_err(context_error)? else {
        return Ok(());
    };
    if pending.context.harness != Harness::Human {
        return Ok(());
    }
    journal.complete_operation(&crate::protocol::ids::OperationId::new(
        pending.operation_id.to_string(),
    ))?;
    contexts
        .abandon_pending(&pending.event_id)
        .map_err(context_error)?;
    Ok(())
}

pub(crate) fn run_me_init<W: Write>(
    mut parsed: ParsedCli,
    operator: bool,
    caller_pane: Option<&str>,
    context: &RuntimeContext,
    paths: &InstancePaths,
    clock: &Arc<dyn Clock>,
    writer: &mut W,
) -> Result<(), RunError> {
    if parsed.cooperative.is_some() {
        return Err(invalid_request(
            "`me init` records the invoking pane as your own seat; --cooperative-* caller \
             selection does not apply",
        ));
    }
    let pane = caller_pane.filter(|pane| !pane.is_empty()).ok_or_else(|| {
        invalid_request(
            "`me init` runs inside your own Herdr pane: HERDR_PANE_ID is not set in this shell",
        )
    })?;
    let pane = parse_pane(pane)?;
    // Never record a person over agent evidence (TRUST-POLICY A4, best
    // effort; the daemon refuses a person's check-in over an agent binding).
    // `--operator` is the local account's explicit override.
    if !operator
        && let Some(evidence) =
            agent_evidence(std::env::vars(), || pane_agent(context, clock, &pane))
    {
        return Err(agent_evidence_refusal(pane.as_str(), &evidence));
    }
    let (instance, _, client) = connect(paths, clock)?;
    let seat = match pane_seat(&client, &parsed.output, &pane, clock.as_ref())? {
        Some(seat) => seat,
        None => resolve_seat(&client, paths, instance, &pane, clock.as_ref())?,
    };
    let selection = CooperativeSelection {
        seat: seat.clone(),
        target: pane.clone(),
        harness: Harness::Human,
        role: Role::TopLevel,
    };
    let inspection = client.call_with_output(
        Command::SeatInspect(SeatInspectQuery {
            seat: seat.clone(),
            page: PageRequest {
                cursor: None,
                limit: 1,
                max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
            },
        }),
        &parsed.output,
        &budget(clock.as_ref(), 5_000),
    )?;
    let CommandResult::SeatInspect(inspection) = inspection else {
        return Err(mapping_error("service returned no seat inspection"));
    };
    let generation = selected_generation(&selection, &inspection)?;
    let contexts = seat_contexts(paths, instance, &seat)?;
    let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
    if operator {
        discard_pending_human_event(&journal, &contexts)?;
    }
    parsed.action = CliAction::Mutation(check_in_spec(
        &contexts, &seat, &pane, generation, operator,
    )?);
    // A person at a terminal gets one line naming their seat; the machine
    // forms (and `--json`) keep the full check-in result.
    let human = super::output::human_active();
    let mut buffered = Vec::new();
    let mut sink: &mut dyn Write = if human { &mut buffered } else { writer };
    run_selected(
        parsed,
        &selection,
        paths,
        instance,
        &client,
        clock.as_ref(),
        &mut sink,
    )
    .map_err(|error| match error {
        RunError::Api(api) if api.code == ErrorCode::Unauthorized => {
            // A deterministic refusal: the frozen event is never replayed, so
            // a later `me init --operator` (or a re-run after the agent
            // leaves) starts a fresh lifecycle.
            if let Err(error) = discard_pending_human_event(&journal, &contexts) {
                return error;
            }
            RunError::Api(api)
        }
        RunError::Api(mut api) if api.code == ErrorCode::Conflict => {
            api.detail = format!(
                "{}; another check-in for this seat is in progress or was interrupted: see \
                 `herdr-threads pending-ops`",
                api.detail
            );
            RunError::Api(api)
        }
        other => other,
    })?;
    // Honor `--operator` for later person-pane commands, only now that the
    // daemon accepted the check-in (TRUST-POLICY A4).
    if operator
        && let Some(current) = contexts.current().map_err(super::context_run_error)?
        && current.harness == Harness::Human
    {
        contexts
            .set_operator_mark(current.execution)
            .map_err(super::context_run_error)?;
    }
    if human {
        writeln!(
            writer,
            "You are seat {} in pane {}; thread commands run here as you.",
            seat.as_str(),
            pane.as_str()
        )?;
        writer.flush()?;
    }
    Ok(())
}
