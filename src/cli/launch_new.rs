//! `launch --new-tab` and `launch --new-terminal`: create the target, then start the harness.
//!
//! Responsibility
//! - Owns: choosing the host through `hc_target`, creating the target through
//!   `CreateTargetPort`, then the ordinary `launch` into a Herdr tab's root pane, or the
//!   gate-only record around a herdr-core's `terminal.run` (TRUST-POLICY H3).
//! - Hides: the order on a core: the record before the terminal, the seat before the creation
//!   is recorded.
//! - Must not: authorize a gate, write the store, or branch on the host type past
//!   `hc_target`'s answer.

use std::{io::Write, sync::Arc};

use super::{
    RunError,
    caller_location::CallerLocator,
    commands::ParsedCli,
    hc_target::{self, Wanted},
    host_client::LaunchHandle,
    invalid_request,
    launch::LaunchRequest,
};
use crate::{
    daemon::paths::{NodePaths, RuntimeContext},
    ports::{
        CreateTargetOutcome, CreateTargetPort, CreateTargetRequest, CreatedTarget, HostCallContext,
        HostPort, HostWitness, LocalClient, Placement,
    },
    protocol::{
        capabilities::LOCAL_LAUNCH_GATE_V1,
        commands::Command,
        handoff::{LocalLaunchGate, LocalLaunchGateAction},
        ids::{HostCallId, HostLocator, HostTargetId, OperationId},
        output::OutputFormat,
        results::{ApiError, CommandResult},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};

/// Budget of the topology reads and the creation, each (ms).
const BUDGET_MS: u64 = 30_000;

/// A `launch` that creates its target first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchNew {
    pub launch: LaunchRequest,
    pub new_tab: Option<String>,
    pub new_terminal: bool,
    pub space: Option<String>,
}

fn budget(clock: &dyn Clock) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(BUDGET_MS)),
        cancellation: Cancellation::default(),
    }
}

// Allowed: the entry point carries the dispatcher's context, as the other launch runners do.
#[allow(clippy::too_many_arguments)]
pub(super) fn run<W: Write>(
    request: &LaunchNew,
    parsed: &ParsedCli,
    caller_pane: Option<&str>,
    site: Option<&CallerLocator>,
    context: &RuntimeContext,
    paths: &NodePaths,
    clock: &Arc<dyn Clock>,
    writer: &mut W,
) -> Result<(), RunError> {
    if parsed.cooperative.is_some() {
        return Err(invalid_request(
            "launch is a local operator command; --cooperative-* caller selection does not apply",
        ));
    }
    let registry = crate::harness::registry::builtins();
    let registration =
        crate::harness::launch::launch_registration(registry, request.launch.harness.occupant())?;
    // Before anything is created: a refused argument leaves no stray tab.
    registration
        .launch_policy()
        .expect("checked provider")
        .validate_native_argv(&request.launch.argv)?;
    let wanted = Wanted {
        new_tab: request.new_tab.is_some(),
        new_terminal: request.new_terminal,
        space: request.space.as_deref(),
        narrowed: false,
        automatic: true,
    };
    match hc_target::select_wanted(&wanted, site, context.host_endpoint.is_some(), &|name| {
        std::env::var_os(name)
    })? {
        Some(target) => on_core(
            request,
            target,
            registration.metadata().id,
            context,
            paths,
            clock,
            parsed,
            writer,
        ),
        None => on_herdr(request, parsed, caller_pane, context, paths, clock, writer),
    }
}

/// What a creation on `handle` is witnessed against.
fn witness_of(handle: &LaunchHandle) -> Result<HostWitness, RunError> {
    handle
        .witness()
        .cloned()
        .ok_or_else(|| invalid_request("a target host reports its witness"))
}

/// Creates the target through `port` and checks what came back against what was asked.
fn create(
    port: &dyn CreateTargetPort,
    witness: HostWitness,
    placement: Placement,
    label: &str,
    gate_argv: Vec<String>,
    clock: &dyn Clock,
) -> Result<CreatedTarget, RunError> {
    let request = CreateTargetRequest {
        correlation: HostCallId::new(uuid::Uuid::new_v4().to_string()),
        placement,
        cwd: std::env::current_dir()?.canonicalize()?,
        label: label.to_owned(),
        focus: false,
        env: Default::default(),
        directory_context: None,
        expected_witness: witness,
        gate_argv,
    };
    let context = HostCallContext {
        budget: budget(clock),
        expected_boot: None,
        expected_epoch: None,
    };
    match port.create_target(&request, &context) {
        CreateTargetOutcome::Created(created) => {
            if created.validate().is_err()
                || created.correlation != request.correlation
                || created.placement != request.placement
                || created.witness != request.expected_witness
            {
                return Err(ApiError::unknown_outcome(
                    "the created target differs from the request",
                )
                .into());
            }
            Ok(*created)
        }
        CreateTargetOutcome::NotSubmitted(error) | CreateTargetOutcome::OutcomeUnknown(error) => {
            Err(error.into())
        }
    }
}

/// A new tab on a Herdr server, then the ordinary `launch` into its root pane.
fn on_herdr<W: Write>(
    request: &LaunchNew,
    parsed: &ParsedCli,
    caller_pane: Option<&str>,
    context: &RuntimeContext,
    paths: &NodePaths,
    clock: &Arc<dyn Clock>,
    writer: &mut W,
) -> Result<(), RunError> {
    let endpoint = super::topology_runtime::herdr_endpoint_io(context, &parsed.output)
        .map_err(|_| super::no_herdr_host("a new tab"))?;
    let herdr = LaunchHandle::herdr(endpoint.clone(), Arc::clone(clock));
    let topology = herdr.host()?.topology(&budget(clock.as_ref()))?;
    let space = match &request.space {
        Some(value) => super::topology_handoff::select_space(&topology, value)?,
        None => caller_pane
            .filter(|pane| !pane.is_empty())
            .and_then(|pane| topology.panes.iter().find(|p| p.target.as_str() == pane))
            .map(|pane| HostTargetId::parse(&pane.space).map_err(invalid_request))
            .transpose()?
            .ok_or_else(|| invalid_request("--new-tab needs --space SPACE outside a Herdr pane"))?,
    };
    let label = request.new_tab.as_deref().unwrap_or_default();
    let handle = LaunchHandle::probed(&endpoint, Arc::clone(clock), &budget(clock.as_ref()))?;
    let created = create(
        handle.create_port(),
        witness_of(&handle)?,
        Placement::NewTab { space },
        label,
        Vec::new(),
        clock.as_ref(),
    )?;
    let root = created
        .root_pane
        .clone()
        .ok_or_else(|| ApiError::unknown_outcome("the created tab reports no root pane"))?;
    let mut launch = request.launch.clone();
    launch.target = root;
    super::run_launch(&launch, parsed, context, paths, clock, writer)
}

/// A terminal on a herdr-core, started under the launch-gate shim on a gate-only record.
// Allowed: the dispatcher's context plus the selection and harness id.
#[allow(clippy::too_many_arguments)]
fn on_core<W: Write>(
    request: &LaunchNew,
    target: super::topology_handoff::TargetSelection,
    harness_id: &'static str,
    context: &RuntimeContext,
    paths: &NodePaths,
    clock: &Arc<dyn Clock>,
    parsed: &ParsedCli,
    writer: &mut W,
) -> Result<(), RunError> {
    if request.launch.harness_binary.is_some() || request.launch.name.is_some() {
        return Err(invalid_request(
            "--harness-binary and --name are not supported for a launch onto a herdr-core",
        ));
    }
    let (_, _, client) = super::connect(paths, clock)?;
    let call_budget = || super::cooperative_budget(clock.as_ref());
    if !client.supports_capability(LOCAL_LAUNCH_GATE_V1, &call_budget()) {
        return Err(invalid_request(&super::older_daemon_refusal(
            &client,
            "a launch onto a herdr-core",
            &call_budget,
        )));
    }
    let handle = LaunchHandle::probed(&target.endpoint, Arc::clone(clock), &call_budget())?;
    let placement = match target.placement {
        Placement::NewTab { space } => {
            let hc = crate::host::hc::HcHost::with_clock(
                crate::host::hc::HcClient::new(&target.endpoint),
                Arc::clone(clock),
            );
            let topology = hc.live_topology(&budget(clock.as_ref()))?;
            Placement::NewTab {
                space: super::topology_handoff::select_space(&topology, space.as_str())?,
            }
        }
        other => other,
    };
    // hc requires a nonblank label and reads it only for a tab.
    let label = request.new_tab.as_deref().unwrap_or(harness_id);
    let id = OperationId::new(uuid::Uuid::new_v4().to_string());
    let plan = crate::sync::frame::LaunchPlan {
        target: crate::sync::frame::LaunchTarget::NewTerminal {
            label: label.to_owned(),
        },
        kind: harness_id.to_owned(),
        argv: request.launch.argv.clone(),
        cwd: None,
    };
    let gate_argv = crate::service::peer_launcher::gate_argv(
        &std::env::current_exe()?,
        &context.state_dir,
        id.as_str(),
        &plan,
        std::env::var_os("PATH").as_deref(),
    )
    .map_err(|why| invalid_request(&why))?;
    let host = HostLocator::parse(
        target
            .endpoint
            .to_str()
            .ok_or_else(|| invalid_request("the herdr-core socket path is not UTF-8"))?,
    )?;
    // `Record` allocates the seat under the daemon's seat budget, which outlives a plain call.
    let record_budget = || {
        let mut budget = call_budget();
        budget.deadline = crate::protocol::time::MonoInstant(
            budget.deadline.0
                + crate::service::launch_gate::GATE_SEAT_BUDGET_MS
                + crate::service::launch_gate::GATE_REPLY_SLACK.as_millis() as u64,
        );
        budget
    };
    let gate = |action: LocalLaunchGateAction| {
        let budget = match action {
            LocalLaunchGateAction::Record { .. } => record_budget(),
            LocalLaunchGateAction::Open => call_budget(),
        };
        client.call(
            Command::LocalLaunchGate(Box::new(LocalLaunchGate {
                id: id.clone(),
                host: host.clone(),
                action,
            })),
            &budget,
        )
    };
    gate(LocalLaunchGateAction::Open)?;
    // Any failure from here returns its error: with no recorded creation the shim is refused
    // at its deadline, so an unknown outcome starts no harness.
    let created = create(
        handle.create_port(),
        witness_of(&handle)?,
        placement,
        label,
        gate_argv,
        clock.as_ref(),
    )?;
    let CommandResult::LocalLaunchGate(Some(seat)) = gate(LocalLaunchGateAction::Record {
        harness: request.launch.harness.occupant(),
        created: Box::new(created.clone()),
    })?
    else {
        return Err(invalid_request("unexpected launch gate record answer"));
    };
    let report = serde_json::json!({
        "seat": seat.as_str(),
        "target": created.target.as_str(),
        "host": target.endpoint.to_string_lossy(),
        "harness": harness_id,
        "outcome": "gate_open",
    });
    let bytes = match parsed.output.format {
        OutputFormat::Json => {
            let mut bytes = serde_json::to_vec(&super::launch::json_envelope(report))
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            bytes.push(b'\n');
            bytes
        }
        OutputFormat::Text => super::setup::render_text(&report).into_bytes(),
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ports::HcPeer,
        protocol::{
            ids::{HostBootId, TerminalId},
            results::ErrorCode,
        },
    };
    use std::sync::Mutex;

    fn witness() -> HostWitness {
        HostWitness::HcCore {
            core_id: "core-a".into(),
            peer: HcPeer {
                pid: 42,
                start_seconds: 7,
                uid: 501,
            },
        }
    }

    fn placement() -> Placement {
        Placement::NewTerminal {
            parent: HostTargetId::new("t1"),
        }
    }

    /// Answers `Created` with the request's own correlation, placement and witness, then
    /// `change` alters the one thing a test wants to differ.
    struct Port<F: Fn(&mut CreatedTarget) + Send + Sync> {
        change: F,
        asked: Mutex<Vec<HostCallId>>,
    }
    impl<F: Fn(&mut CreatedTarget) + Send + Sync> CreateTargetPort for Port<F> {
        fn create_target(
            &self,
            request: &CreateTargetRequest,
            _: &HostCallContext,
        ) -> CreateTargetOutcome {
            self.asked.lock().unwrap().push(request.correlation.clone());
            let mut created = CreatedTarget {
                correlation: request.correlation.clone(),
                placement: request.placement.clone(),
                target: HostTargetId::new("t2"),
                root_pane: None,
                terminal: TerminalId::new("t2"),
                host_incarnation: HostBootId::new("hc-core:core-a:pid=42:start=7:uid=501"),
                witness: request.expected_witness.clone(),
            };
            (self.change)(&mut created);
            CreateTargetOutcome::Created(Box::new(created))
        }
    }

    fn created_with(
        change: impl Fn(&mut CreatedTarget) + Send + Sync,
    ) -> Result<CreatedTarget, RunError> {
        let port = Port {
            change,
            asked: Mutex::default(),
        };
        let clock = crate::app::SystemClock::new();
        create(&port, witness(), placement(), "worker", Vec::new(), &clock)
    }

    fn unknown_outcome(result: Result<CreatedTarget, RunError>) -> ApiError {
        match result {
            Err(RunError::Api(error)) => error,
            other => panic!("an unknown outcome, got {other:?}"),
        }
    }

    // Kills: a `create` that accepts a creation of another call (the correlation comparison
    // removed). Everything else in the answer matches the request.
    #[test]
    fn created_target_with_a_foreign_correlation_is_unknown_outcome() {
        let error = unknown_outcome(created_with(|created| {
            created.correlation = HostCallId::new("another-call");
        }));
        assert_eq!(error.code, ErrorCode::UnknownOutcome);
        assert!(
            error.detail.contains("differs from the request"),
            "{}",
            error.detail
        );
    }

    // Kills: a `create` that accepts a creation placed elsewhere than asked (the placement
    // comparison removed). The creation stays valid: its target differs from the new parent.
    #[test]
    fn created_target_with_a_foreign_placement_is_unknown_outcome() {
        let error = unknown_outcome(created_with(|created| {
            created.placement = Placement::NewTerminal {
                parent: HostTargetId::new("t7"),
            };
        }));
        assert_eq!(error.code, ErrorCode::UnknownOutcome);
        assert!(
            error.detail.contains("differs from the request"),
            "{}",
            error.detail
        );
    }

    // Kills: a `create` that accepts a creation witnessed by another core (the witness
    // comparison removed). The creation stays self-consistent: its incarnation names the
    // other peer.
    #[test]
    fn created_target_with_a_foreign_witness_is_unknown_outcome() {
        let error = unknown_outcome(created_with(|created| {
            created.witness = HostWitness::HcCore {
                core_id: "core-b".into(),
                peer: HcPeer {
                    pid: 43,
                    start_seconds: 8,
                    uid: 501,
                },
            };
            created.host_incarnation = HostBootId::new("hc-core:core-b:pid=43:start=8:uid=501");
        }));
        assert_eq!(error.code, ErrorCode::UnknownOutcome);
        assert!(
            error.detail.contains("differs from the request"),
            "{}",
            error.detail
        );
    }

    // Kills: a `create` that skips the creation's own validation (the `validate` call removed).
    // Correlation, placement and witness all match; the creation answers its own parent.
    #[test]
    fn invalid_created_target_is_unknown_outcome() {
        let error = unknown_outcome(created_with(|created| {
            created.target = HostTargetId::new("t1");
        }));
        assert_eq!(error.code, ErrorCode::UnknownOutcome);
        assert!(
            error.detail.contains("differs from the request"),
            "{}",
            error.detail
        );
    }

    // The control: an answer that matches the request in every field is returned as it is.
    #[test]
    fn matching_created_target_is_returned() {
        let created = created_with(|_| {}).expect("a matching creation");
        assert_eq!(created.target, HostTargetId::new("t2"));
    }
}
