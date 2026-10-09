//! Concrete capability-gated public composition over reviewed immutable consumers.
use super::{
    RunError,
    actor_route::InvocationActor,
    commands::{CliAction, ParsedCli},
    handoff_delivery::FrozenDeliveryClient,
    journal::{IntentScope, Journal, OriginalActor, SemanticMutation},
};
use crate::{
    daemon::paths::{InstancePaths, RuntimeContext},
    host::native::NativeCli,
    ports::{BootstrapObserver, HostCallContext, LocalClient},
    protocol::{
        authority::CallerClaim,
        commands::Command,
        handoff::*,
        output::OutputSpec,
        results::{CommandResult, ErrorCode},
        time::Clock,
    },
};
use std::{io::Write, sync::Arc};
// Keep public CLI writer variants from duplicating the compound coordinators.
struct ForwardWriter<'a>(&'a mut dyn Write);
impl Write for ForwardWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

fn runtime(output: &OutputSpec) -> Result<RuntimeContext, RunError> {
    Ok(
        super::instance::resolve_context(&super::instance::InstanceInputs::from_process(
            output
                .context
                .state_dir
                .as_ref()
                .map(std::path::PathBuf::from),
            output.context.host.as_ref().map(std::path::PathBuf::from),
        ))?
        .0,
    )
}
fn namespace(selected: &RuntimeContext, instance: uuid::Uuid) -> HandoffNamespace {
    HandoffNamespace {
        instance: instance.to_string(),
        state_dir: selected.state_dir.clone(),
        host_endpoint: selected.host_endpoint.clone(),
    }
}
fn require_capability<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    match client.call(Command::Capabilities, &super::cooperative_budget(clock))? {
        CommandResult::Capabilities(c)
            if c.capabilities
                .iter()
                .any(|v| v == crate::protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1) =>
        {
            Ok(())
        }
        _ => Err(super::unsupported(
            "daemon lacks guarded handoff capability",
        )),
    }
}
fn original_kind(semantic: &SemanticMutation) -> &SemanticMutation {
    match semantic {
        SemanticMutation::Frozen { mutation, .. } => mutation,
        v => v,
    }
}
pub(crate) fn is_compound(semantic: &SemanticMutation) -> bool {
    matches!(
        original_kind(semantic),
        SemanticMutation::HandoffBootstrap(_) | SemanticMutation::HandoffDelivery(_)
    )
}
fn agent_claim(claim: &CallerClaim) -> Result<(), RunError> {
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    if super::journal::classify_original_claim(&scope, claim)? != OriginalActor::Agent
        || claim.role != crate::protocol::authority::CallerRole::TopLevel
    {
        return Err(super::invalid_request(
            "new handoff requires original top-level agent",
        ));
    }
    Ok(())
}
/// Historical namespace authority is daemon selected, before live caller/host reads.
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_completed<C: LocalClient + ?Sized>(
    parsed: &ParsedCli,
    journal: &Journal,
    instance: uuid::Uuid,
    selected: &RuntimeContext,
    client: &C,
    clock: &dyn Clock,
    writer: &mut dyn Write,
) -> Result<bool, RunError> {
    let mut sink = ForwardWriter(writer);
    let writer = &mut sink;
    let CliAction::Retry(recovery) = &parsed.action else {
        return Ok(false);
    };
    let original = super::retry::load_original_for_actor(journal, recovery.as_str())?;
    let reference = &original.header.reference;
    let ns = namespace(selected, instance);
    match original_kind(&original.semantic) {
        SemanticMutation::HandoffBootstrap(_) => {
            require_capability(client, clock)?;
            super::topology_handoff::try_completed(
                journal,
                reference,
                &ns,
                client,
                clock,
                &parsed.output,
                writer,
            )
        }
        SemanticMutation::HandoffDelivery(_) => {
            require_capability(client, clock)?;
            let guarded = FrozenDeliveryClient::new(client, &original)?;
            match guarded.query(false, clock) {
                Ok(CommandResult::Handoff(current)) if current.state == HandoffState::Completed => {
                    super::retry::run_delivery_retry_to_writer(
                        journal,
                        reference,
                        parsed.actor,
                        &ns,
                        &guarded,
                        clock,
                        &parsed.output,
                        writer,
                    )?;
                    Ok(true)
                }
                Ok(CommandResult::Handoff(_)) => Ok(false),
                Err(RunError::Api(e)) if e.code == ErrorCode::NotFound => Ok(false),
                Err(e) => Err(e),
                _ => Err(super::invalid_request("unexpected scoped delivery status")),
            }
        }
        _ => Ok(false),
    }
}

pub(crate) fn run(
    parsed: ParsedCli,
    claim: CallerClaim,
    journal: &Journal,
    paths: &InstancePaths,
    writer: &mut dyn Write,
) -> Result<(), RunError> {
    let mut sink = ForwardWriter(writer);
    let writer = &mut sink;
    let clock: Arc<dyn Clock> = Arc::new(super::SystemClock::new());
    let selected = runtime(&parsed.output)?;
    let (instance, _, client) = super::connect(paths, &clock)?;
    let ns = namespace(&selected, instance);
    agent_claim(&claim)?;
    require_capability(&client, clock.as_ref())?;
    let host = NativeCli::new(selected.host_endpoint.clone(), clock.clone());
    let mut output = parsed.output.clone();
    output.context.state_dir = Some(selected.state_dir.to_string_lossy().into_owned());
    output.context.host = Some(selected.host_endpoint.to_string_lossy().into_owned());
    let reference = match &parsed.action {
        CliAction::TopologyHandoff(request) if request.existing => {
            let topology = host.topology(&super::cooperative_budget(clock.as_ref()))?;
            let plan = super::handoff_delivery::prepare(
                request,
                super::handoff_delivery::Preparation {
                    claim: &claim,
                    namespace: &ns,
                    topology: &topology,
                    client: &client,
                    clock: clock.as_ref(),
                },
            )?;
            let scope = IntentScope::Cooperative {
                instance: claim.instance.clone(),
                seat: claim.seat.clone(),
            };
            let semantic = SemanticMutation::freeze(
                SemanticMutation::HandoffDelivery(Box::new(plan.clone())),
                claim.clone(),
            )?;
            use sha2::{Digest, Sha256};
            let digest = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&semantic).map_err(std::io::Error::other)?)
            );
            let guarded = FrozenDeliveryClient::from_plan(
                &client,
                scope,
                claim.clone(),
                digest,
                plan.clone(),
            )?;
            guarded.query(true, clock.as_ref())?;
            let reference = super::handoff_delivery::publish(journal, plan, claim, clock.as_ref())?;
            let original = super::handoff_delivery::load_original(journal, &reference)?;
            let guarded = FrozenDeliveryClient::new(&client, &original)?;
            super::retry::run_delivery_retry_to_writer(
                journal,
                &reference,
                parsed.actor,
                &ns,
                &guarded,
                clock.as_ref(),
                &output,
                writer,
            )?;
            return Ok(());
        }
        CliAction::TopologyHandoff(request) => {
            let topology = host.topology(&super::cooperative_budget(clock.as_ref()))?;
            let cwd = std::env::current_dir()?;
            let registry = crate::harness::registry::builtins();
            let harness = match request.kind.as_deref() {
                Some("codex") => crate::harness::context::Harness::Codex,
                Some("claude") => crate::harness::context::Harness::Claude,
                _ => return Err(super::invalid_request("invalid bootstrap harness")),
            };
            let options =
                super::launch::native_options_env(registry, harness)?.and_then(std::env::var_os);
            let identity = super::topology_handoff::prepare(
                request,
                super::topology_handoff::Preparation {
                    claim: &claim,
                    namespace: &ns,
                    topology: &topology,
                    invocation_cwd: &cwd,
                    options,
                    client: &client,
                    clock: clock.as_ref(),
                },
            )?;
            // Validate the actual selected executable, installed configuration and
            // full frozen Root append input before allocating durable/native work.
            // The new target does not exist yet; input-only preparation never
            // resolves it. Canonical target admission remains after creation.
            let mut env = super::setup::SetupEnv::from_process(&output)?;
            env.state_dir = Some(selected.state_dir.clone());
            env.host_endpoint = Some(selected.host_endpoint.clone());
            let seats =
                super::launch::DaemonSeatResolver::new(&client, journal, instance, clock.as_ref());
            let shell = super::launch::SystemShellProbe::from_process();
            let effective = super::topology_handoff::preflight_launch_request(&identity)?;
            let expected = crate::harness::launch::compose_native_argv(
                identity.payload.launch.harness,
                effective.argv.clone(),
                vec![],
            )?;
            let input = super::launch::prepare_native_input_with_registry(
                registry,
                &effective,
                &super::launch::LaunchParts {
                    env: &env,
                    host: &host,
                    seats: &seats,
                    handoff: &client,
                    clock: clock.as_ref(),
                    record_dir: Some(&paths.instance_dir),
                    shell_probe: &shell,
                },
            )?;
            if input.argv != expected {
                return Err(super::invalid_request(
                    "selected native input differs from frozen bootstrap append contract",
                ));
            }
            // Namespace is checked by the actual selected daemon even before a new row exists.
            match client.call(
                Command::BootstrapStatus(Box::new(BootstrapStatus {
                    identity: identity.clone(),
                })),
                &super::cooperative_budget(clock.as_ref()),
            ) {
                Err(e) if e.code == ErrorCode::NotFound => {}
                Ok(_) => {
                    return Err(super::invalid_request(
                        "new bootstrap compound already exists",
                    ));
                }
                Err(e) => return Err(e.into()),
            }
            let context = HostCallContext {
                budget: super::cooperative_budget(clock.as_ref()),
                expected_boot: None,
                expected_epoch: None,
            };
            let pane = host.observe_bootstrap_target(&claim.target, &context)?;
            super::topology_handoff::preflight_progress_capacity(&identity, pane.witness())?;
            super::topology_handoff::publish(journal, &identity, clock.utc_now().0)?
        }
        CliAction::Retry(recovery) => {
            let original = super::retry::load_original_for_actor(journal, recovery.as_str())?;
            if original.header.scope
                != (IntentScope::Cooperative {
                    instance: claim.instance.clone(),
                    seat: claim.seat.clone(),
                })
                || original.semantic.frozen_claim() != Some(&claim)
            {
                return Err(super::invalid_request(
                    "live retry differs from original caller",
                ));
            }

            if let SemanticMutation::HandoffDelivery(_) = original_kind(&original.semantic) {
                let guarded = FrozenDeliveryClient::new(&client, &original)?;
                super::retry::run_delivery_retry_to_writer(
                    journal,
                    &original.header.reference,
                    parsed.actor,
                    &ns,
                    &guarded,
                    clock.as_ref(),
                    &output,
                    writer,
                )?;
                return Ok(());
            }
            original.header.reference
        }
        _ => return Err(super::invalid_request("not a topology handoff")),
    };
    let original = super::topology_handoff::load_original(journal, &reference)?;
    let identity = super::topology_handoff::bootstrap_identity(&original)?;
    crate::store::topology_handoff::encode_identity(&ns, &identity)?;
    // Last canonical status this invocation observed before a continuation
    // failure; None means unknown, never inferred from publication.
    let mut observed: Option<BootstrapResult> = None;
    let continuation = (|| -> Result<_, RunError> {
        let status = match client.call(
            Command::BootstrapStatus(Box::new(BootstrapStatus {
                identity: identity.clone(),
            })),
            &super::cooperative_budget(clock.as_ref()),
        ) {
            Ok(CommandResult::Bootstrap(v)) => Some(*v),
            Err(e) if e.code == ErrorCode::NotFound => None,
            Err(e) => return Err(e.into()),
            _ => return Err(super::invalid_request("unexpected bootstrap status")),
        };
        observed = status.clone().filter(|v| v.compound == identity.compound);
        let request = {
            let _lock = super::handoff::lock(journal, &reference)?;
            // Malformed retained progress is a refusal, never a trusted report.
            match super::topology_handoff::saved_native_request(journal, &reference, &identity) {
                Ok(request) => request,
                Err(error) => return Ok(Err(error)),
            }
        };
        if status
            .as_ref()
            .is_some_and(|v| v.state == BootstrapState::Cancelled)
        {
            return Ok(Err(super::invalid_request("bootstrap is cancelled")));
        }
        let retained_witness = status
            .as_ref()
            .and_then(|v| v.creation.as_ref())
            .map(|v| v.witness.clone())
            .or_else(|| request.as_ref().map(|v| v.expected_witness.clone()));
        if status
            .as_ref()
            .is_some_and(|v| v.state == BootstrapState::PossibleCreation)
            && retained_witness.is_none()
        {
            // Already-presented PossibleCreation route keeps its own report;
            // its own output failure is never presented a second time.
            if let Err(error) = super::topology_handoff::write_pending(
                &reference,
                &identity,
                status.as_ref().unwrap(),
                None,
                "creation",
                &output,
                writer,
            ) {
                return Ok(Err(error));
            }
            return Ok(Err(super::topology_handoff::creation_unknown(
                &reference,
                &identity,
                status.as_ref().unwrap().attempt,
            )));
        }
        // Historical witness is used only for a coordinator branch that cannot resubmit.
        // Prepared attempts always capture a real fresh witnessed native read.
        let witness = if status.as_ref().is_some_and(|v| {
            matches!(
                v.state,
                BootstrapState::PossibleCreation
                    | BootstrapState::Created
                    | BootstrapState::Attached
            )
        }) {
            match retained_witness {
                Some(witness) => witness,
                None => {
                    return Ok(Err(super::invalid_request(
                        "canonical continuation lacks retained witness",
                    )));
                }
            }
        } else {
            let context = HostCallContext {
                budget: super::cooperative_budget(clock.as_ref()),
                expected_boot: None,
                expected_epoch: None,
            };
            host.observe_bootstrap_target(&identity.claim.target, &context)?
                .witness()
                .clone()
        };
        super::topology_handoff::preflight_progress_capacity(&identity, &witness)?;
        let submission_context = HostCallContext {
            budget: super::cooperative_budget(clock.as_ref()),
            expected_boot: None,
            expected_epoch: None,
        };
        let env = super::setup::SetupEnv::from_process(&output)?;
        Ok(Ok((witness, submission_context, env)))
    })();
    let (witness, submission_context, mut env) = match continuation {
        Ok(Ok(prepared)) => prepared,
        // Refusals or routes that already presented their own report.
        Ok(Err(error)) => return Err(error),
        Err(error) => {
            // Admitted post-publication continuation failure before the inner
            // writer: present only what was observed, after the unchanged actor
            // gate. Authority refusals and the actor gate stay silent.
            if super::topology_handoff::reportable_failure(&error)
                && super::retry::preflight_original_actor(
                    journal.root(),
                    &reference.recovery_ref(),
                    parsed.actor,
                    &output.context,
                )
                .is_ok_and(|actor| actor == OriginalActor::Agent)
                && observed.as_ref().is_none_or(|v| {
                    !matches!(
                        v.state,
                        BootstrapState::Completed | BootstrapState::Cancelled
                    )
                })
            {
                super::topology_handoff::write_pending_observed(
                    &reference,
                    &identity,
                    observed.as_ref(),
                    None,
                    "creation",
                    &output,
                    writer,
                )?;
            }
            return Err(error);
        }
    };
    env.state_dir = Some(selected.state_dir);
    env.host_endpoint = Some(selected.host_endpoint);
    let seats = super::launch::DaemonSeatResolver::new(&client, journal, instance, clock.as_ref());
    let shell = super::launch::SystemShellProbe::from_process();
    let mut launcher = super::handoff::NativeLauncher {
        registry: crate::harness::registry::builtins(),
        parts: super::launch::LaunchParts {
            env: &env,
            host: &host,
            seats: &seats,
            handoff: &client,
            clock: clock.as_ref(),
            record_dir: Some(&paths.instance_dir),
            shell_probe: &shell,
        },
    };
    super::retry::run_bootstrap_retry_to_writer(
        journal,
        &reference,
        parsed.actor,
        &ns,
        &client,
        &host,
        &mut launcher,
        clock.as_ref(),
        super::topology_handoff::BootstrapSubmissionInputs {
            witness: &witness,
            context: &submission_context,
        },
        &output,
        writer,
    )?;
    Ok(())
}

pub(crate) fn run_operator(
    parsed: &ParsedCli,
    selected: &RuntimeContext,
    paths: &InstancePaths,
    clock: &Arc<dyn Clock>,
    writer: &mut dyn Write,
) -> Result<bool, RunError> {
    let mut sink = ForwardWriter(writer);
    let writer = &mut sink;
    if !matches!(
        parsed.action,
        CliAction::TopologyRecover(_) | CliAction::Retry(_)
    ) {
        return Ok(false);
    }
    let journal = Journal::open(paths.instance_dir.join("intents"))?;
    let original = match &parsed.action {
        CliAction::TopologyRecover(_) => None,
        CliAction::Retry(recovery) => {
            let original = super::retry::load_original_for_actor(&journal, recovery.as_str())?;
            if !matches!(
                original.semantic,
                SemanticMutation::OperatorRecoverBootstrap(_)
            ) {
                return Ok(false);
            }
            Some(original)
        }
        _ => return Ok(false),
    };
    if parsed.actor != InvocationActor::Human {
        return Err(super::invalid_request(
            "recovery requires immediate human namespace",
        ));
    }
    let (instance, _, client) = super::connect(paths, clock)?;
    let ns = namespace(selected, instance);
    super::topology_recover::require_capability(&client, clock.as_ref())?;
    let reference = if let Some(original) = original {
        original.header.reference
    } else {
        let CliAction::TopologyRecover(request) = &parsed.action else {
            unreachable!()
        };
        let original = super::retry::load_original_for_actor(&journal, request.reference.as_str())?;
        if super::journal::classify_original_actor(&original.header.scope, &original.semantic)?
            != OriginalActor::Agent
        {
            return Err(super::invalid_request(
                "recovery needs original agent bootstrap",
            ));
        }
        let identity = super::topology_handoff::bootstrap_identity(&original)?;
        crate::store::topology_handoff::encode_identity(&ns, &identity)?;
        let lock = super::handoff::lock(&journal, &original.header.reference)?;
        let saved_request = super::topology_handoff::saved_native_request(
            &journal,
            &original.header.reference,
            &identity,
        )?;
        let CommandResult::Bootstrap(status) = client.call(
            Command::BootstrapStatus(Box::new(BootstrapStatus {
                identity: identity.clone(),
            })),
            &super::cooperative_budget(clock.as_ref()),
        )?
        else {
            return Err(super::invalid_request("unexpected recovery status"));
        };
        let disposition = match &request.disposition {
            super::topology_recover::Assertion::CreatedPane(target) => {
                let host = NativeCli::new(selected.host_endpoint.clone(), clock.clone());
                let context = HostCallContext {
                    budget: super::cooperative_budget(clock.as_ref()),
                    expected_boot: None,
                    expected_epoch: None,
                };
                let pane = host.observe_bootstrap_target(target, &context)?;
                let observation = pane.observation();
                BootstrapRecoveryDisposition::CreatedPane {
                    evidence: crate::ports::CreatedTab {
                        correlation: saved_request
                            .as_ref()
                            .map(|v| v.correlation.clone())
                            .unwrap_or_else(|| {
                                crate::protocol::ids::HostCallId::new(
                                    uuid::Uuid::new_v4().to_string(),
                                )
                            }),
                        workspace: pane.workspace().clone(),
                        tab: pane.tab().clone(),
                        root_pane: observation.target.clone(),
                        terminal: observation.terminal.clone().ok_or_else(|| {
                            super::invalid_request("recovery pane lacks terminal")
                        })?,
                        host_incarnation: observation.host_boot.clone(),
                        witness: pane.witness().clone(),
                    },
                    structural_reference: observation.call_id.clone(),
                }
            }
            super::topology_recover::Assertion::NotCreated => {
                BootstrapRecoveryDisposition::NotCreated {
                    quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
                }
            }
            super::topology_recover::Assertion::Cancelled { reason } => {
                BootstrapRecoveryDisposition::Cancelled {
                    reason: reason.clone(),
                    quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                    child_guard: BootstrapCancellationGuard {
                        attached_child: status.attachment.as_ref().map(|v| v.handoff.clone()),
                    },
                }
            }
        };
        let plan = super::topology_recover::prepare(
            &journal,
            request,
            disposition,
            unsafe { libc::geteuid() },
            &ns,
            &status,
        )?;
        let reference = super::topology_recover::publish_under_guard(
            &journal,
            &plan,
            &ns,
            clock.utc_now().0,
            &lock,
        )?;
        super::topology_recover::retry_under_guard(
            &journal,
            &reference,
            &ns,
            &client,
            clock.as_ref(),
            &parsed.output,
            writer,
            &lock,
        )?;
        return Ok(true);
    };
    super::retry::run_topology_recovery_retry_to_writer(
        &journal,
        &reference,
        parsed.actor,
        &ns,
        &client,
        clock.as_ref(),
        &parsed.output,
        writer,
    )?;
    Ok(true)
}
