//! New-tab V2 producer. Canonical parent and child commands retain all authority;
//! these local records only preserve the original native submission boundary.
use super::{BootstrapSubmissionInputs, RunError};
use crate::{
    cli::{
        actor_route::InvocationActor,
        handoff::HandoffLauncher,
        journal::{IntentRef, Journal},
    },
    ports::{CreateTabOutcome, CreateTabPort, CreateTabRequest, LocalClient},
    protocol::{
        commands::Command,
        handoff::*,
        ids::{HostCallId, ThreadId},
        output::{OutputFormat, OutputSpec},
        results::{ApiError, CommandResult, ErrorCode},
        time::Clock,
    },
};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentProgress {
    version: u32,
    identity: AutomaticBootstrapIdentity,
    attempt: BootstrapAttempt,
    possible_creation: bool,
    request: Option<CreateTabRequest>,
    creation: Option<crate::ports::CreatedTab>,
    not_submitted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChildProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection: Option<crate::protocol::results::DirectThreadEnsured>,
    version: u32,
    identity: AutomaticBootstrapIdentity,
    attachment: AutomaticBootstrapAttachment,
    thread: Option<ThreadId>,
    message: Option<CommandResult>,
    possible_start: bool,
    launch: Option<serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Terminal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection: Option<crate::protocol::results::DirectThreadEnsured>,
    version: u32,
    original: String,
    completed: AutomaticBootstrapResult,
    message: Option<CommandResult>,
}

fn invalid(detail: &str) -> RunError {
    super::super::invalid_request(detail)
}
fn budget(clock: &dyn Clock) -> crate::protocol::time::CallBudget {
    super::super::cooperative_budget(clock)
}
fn parent_request(id: &AutomaticBootstrapIdentity, action: AutomaticBootstrapAction) -> Command {
    Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
        identity: id.clone(),
        action,
    }))
}
fn call<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    command: Command,
) -> Result<CommandResult, RunError> {
    Ok(client.call(command, &budget(clock))?)
}
fn status<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    id: &AutomaticBootstrapIdentity,
) -> Result<AutomaticBootstrapResult, RunError> {
    parent_result(
        id,
        call(
            client,
            clock,
            parent_request(id, AutomaticBootstrapAction::Status),
        )?,
    )
}
fn parent_result(
    id: &AutomaticBootstrapIdentity,
    result: CommandResult,
) -> Result<AutomaticBootstrapResult, RunError> {
    let CommandResult::AutomaticBootstrap(value) = result else {
        return Err(invalid("unexpected automatic bootstrap result"));
    };
    if value.identity != *id {
        return Err(invalid("automatic bootstrap canonical identity differs"));
    }
    Ok(*value)
}
fn exact_namespace(
    id: &AutomaticBootstrapIdentity,
    namespace: &HandoffNamespace,
) -> Result<(), RunError> {
    let saved = &id.plan.payload.handoff.namespace;
    if saved.instance != namespace.instance
        || saved.state_dir.as_os_str() != namespace.state_dir.as_os_str()
        || saved.host_endpoint.as_os_str() != namespace.host_endpoint.as_os_str()
    {
        return Err(invalid("automatic bootstrap selected namespace differs"));
    }
    Ok(())
}
fn parent_path(journal: &Journal, reference: &IntentRef) -> PathBuf {
    journal.root().join(format!(
        "automatic-bootstrap-{}.progress",
        reference.operation.as_str()
    ))
}
fn child_path(journal: &Journal, reference: &IntentRef) -> PathBuf {
    journal.root().join(format!(
        "automatic-child-{}.progress",
        reference.operation.as_str()
    ))
}
fn terminal_path(journal: &Journal, reference: &IntentRef) -> PathBuf {
    journal.root().join(format!(
        "automatic-bootstrap-{:020}-{}.terminal",
        reference.ordinal,
        reference.operation.as_str()
    ))
}
fn save_record<T: Serialize>(
    journal: &Journal,
    path: &std::path::Path,
    value: &T,
) -> Result<(), RunError> {
    if serde_json::to_vec(value)
        .map_err(std::io::Error::other)?
        .len()
        > super::MAX_LINKED_LOCAL_BYTES
    {
        return Err(invalid("oversized automatic bootstrap local progress"));
    }
    super::super::handoff::save_progress_at(journal, path, value)?;
    Ok(())
}
fn load_parent(
    journal: &Journal,
    reference: &IntentRef,
    id: &AutomaticBootstrapIdentity,
) -> Result<Option<ParentProgress>, RunError> {
    let Some(bytes) = super::read_linked_bytes(
        &parent_path(journal, reference),
        super::MAX_LINKED_LOCAL_BYTES,
    )?
    else {
        return Ok(None);
    };
    let value: ParentProgress = super::decode_linked(&bytes, super::MAX_LINKED_LOCAL_BYTES)?;
    if value.version != 2
        || value.identity != *id
        || (!value.possible_creation
            && (value.request.is_some() || value.creation.is_some() || value.not_submitted))
        || (value.not_submitted && (value.request.is_none() || value.creation.is_some()))
        || value.request.as_ref().is_some_and(|r| {
            r.workspace != id.plan.payload.workspace
                || r.cwd != id.plan.payload.cwd
                || r.label != id.plan.payload.label
                || r.focus
                || !r.env.is_empty()
                || r.expected_witness.endpoint.as_os_str()
                    != id.plan.payload.handoff.namespace.host_endpoint.as_os_str()
        })
        || value.creation.as_ref().is_some_and(|created| {
            created.workspace != id.plan.payload.workspace
                || created.witness.endpoint.as_os_str()
                    != id.plan.payload.handoff.namespace.host_endpoint.as_os_str()
                || value.request.as_ref().is_some_and(|r| {
                    r.correlation != created.correlation || r.expected_witness != created.witness
                })
        })
    {
        return Err(invalid(
            "automatic bootstrap local creation evidence differs",
        ));
    }
    Ok(Some(value))
}
pub(super) fn saved_native_request(
    journal: &Journal,
    reference: &IntentRef,
    id: &AutomaticBootstrapIdentity,
) -> Result<Option<crate::ports::CreateTabRequest>, RunError> {
    Ok(load_parent(journal, reference, id)?.and_then(|value| value.request))
}
fn load_child(
    journal: &Journal,
    reference: &IntentRef,
    id: &AutomaticBootstrapIdentity,
    attachment: &AutomaticBootstrapAttachment,
) -> Result<ChildProgress, RunError> {
    let Some(bytes) = super::read_linked_bytes(
        &child_path(journal, reference),
        super::MAX_LINKED_LOCAL_BYTES,
    )?
    else {
        return Ok(ChildProgress {
            selection: None,
            version: 2,
            identity: id.clone(),
            attachment: attachment.clone(),
            thread: None,
            message: None,
            possible_start: false,
            launch: None,
        });
    };
    let value: ChildProgress = super::decode_linked(&bytes, super::MAX_LINKED_LOCAL_BYTES)?;
    if value.version != 2
        || value.selection.as_ref().is_some_and(|actual| {
            actual.validate().is_err()
                || Some(&actual.thread) != value.thread.as_ref()
                || actual.recipient != attachment.resolved_seat
                || id.plan.payload.handoff.policy.is_none()
        })
        || value.identity != *id
        || value.attachment != *attachment
        || value.thread.as_ref().is_some_and(|thread| {
            id.child(&attachment.created, attachment.resolved_seat.clone())
                .is_err()
                || thread.as_str().is_empty()
        })
        || value
            .message
            .as_ref()
            .is_some_and(|v| !matches!(v, CommandResult::MessageSent(_)))
        || (value.possible_start && (value.thread.is_none() || value.message.is_none()))
        || (value.launch.is_some() && !value.possible_start)
    {
        return Err(invalid("automatic bootstrap child progress differs"));
    }
    Ok(value)
}
fn save_parent(
    journal: &Journal,
    reference: &IntentRef,
    value: &ParentProgress,
) -> Result<(), RunError> {
    save_record(journal, &parent_path(journal, reference), value)
}
fn save_child(
    journal: &Journal,
    reference: &IntentRef,
    value: &ChildProgress,
) -> Result<(), RunError> {
    save_record(journal, &child_path(journal, reference), value)
}

fn unknown(reference: &IntentRef, attempt: BootstrapAttempt) -> RunError {
    ApiError::unknown_outcome(format!("automatic bootstrap {} attempt {} has uncertain topology submission; inspect the exact namespace before recovery; no automatic creation", reference.recovery_ref(), attempt.get())).into()
}
fn resolve_and_attach<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    id: &AutomaticBootstrapIdentity,
    current: AutomaticBootstrapResult,
) -> Result<AutomaticBootstrapResult, RunError> {
    let created = current
        .creation
        .as_ref()
        .ok_or_else(|| invalid("automatic canonical creation missing"))?;
    let recipient = match call(
        client,
        clock,
        parent_request(
            id,
            AutomaticBootstrapAction::Resolve {
                attempt: current.attempt,
            },
        ),
    )? {
        CommandResult::SeatResolved(value) => value,
        _ => return Err(invalid("unexpected automatic bootstrap resolution")),
    };
    let attachment =
        AutomaticBootstrapAttachment::derive(id, current.attempt, created.clone(), recipient)
            .map_err(invalid)?;
    if current
        .attachment
        .as_ref()
        .is_some_and(|saved| saved != &attachment)
    {
        return Err(invalid("automatic canonical attachment differs"));
    }
    let result = call(
        client,
        clock,
        parent_request(
            id,
            AutomaticBootstrapAction::Attach(Box::new(attachment.clone())),
        ),
    );
    let result = match result {
        Ok(value) => parent_result(id, value)?,
        Err(error) => match status(client, clock, id) {
            Ok(saved) if saved.attachment.as_ref() == Some(&attachment) => saved,
            _ => return Err(error),
        },
    };
    if result.attachment.as_ref() != Some(&attachment)
        || !matches!(
            result.state,
            BootstrapState::Attached | BootstrapState::Completed
        )
    {
        return Err(invalid("automatic bootstrap attached result differs"));
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn resume_to_attachment<C: LocalClient + ?Sized, N: CreateTabPort + ?Sized>(
    journal: &Journal,
    reference: &IntentRef,
    namespace: &HandoffNamespace,
    client: &C,
    native: &N,
    clock: &dyn Clock,
    submission: BootstrapSubmissionInputs<'_>,
) -> Result<AutomaticBootstrapResult, RunError> {
    let pending = super::load_automatic_original(journal, reference)?;
    let id = super::automatic_bootstrap_identity(&pending)?;
    exact_namespace(&id, namespace)?;
    let _lock = super::super::handoff::lock(journal, reference)?;
    if id.plan.payload.handoff.policy.is_some() {
        let completed =
            matches!(status(client, clock, &id), Ok(s) if s.state == BootstrapState::Completed);
        if !completed {
            super::super::handoff_delivery::require_policy_capabilities(
                client,
                clock,
                id.plan.payload.handoff.policy.as_ref(),
            )?;
        }
    }
    let mut local = load_parent(journal, reference, &id)?;
    let begun = call(
        client,
        clock,
        parent_request(&id, AutomaticBootstrapAction::Begin),
    );
    let current = match begun {
        Ok(result) => parent_result(&id, result)?,
        Err(error) => match status(client, clock, &id) {
            Ok(saved) => saved,
            Err(_) => return Err(error),
        },
    };
    if matches!(
        current.state,
        BootstrapState::Attached | BootstrapState::Completed
    ) {
        return Ok(current);
    }
    if current.state == BootstrapState::Cancelled {
        return Err(invalid("automatic bootstrap is cancelled"));
    }
    if let Some(progress) = &local {
        if progress.attempt > current.attempt
            || (progress.attempt < current.attempt && progress.creation.is_some())
        {
            return Err(invalid(
                "automatic local attempt contradicts canonical attempt",
            ));
        }
        if progress.attempt < current.attempt {
            local = None;
        }
    }
    let mut local = local.unwrap_or_else(|| ParentProgress {
        version: 2,
        identity: id.clone(),
        attempt: current.attempt,
        possible_creation: false,
        request: None,
        creation: None,
        not_submitted: false,
    });
    if current.creation.is_some() {
        return resolve_and_attach(client, clock, &id, current);
    }
    if let Some(created) = &local.creation {
        let result = call(
            client,
            clock,
            parent_request(
                &id,
                AutomaticBootstrapAction::RecordCreated {
                    attempt: local.attempt,
                    evidence: created.clone(),
                },
            ),
        );
        let saved = match result {
            Ok(value) => parent_result(&id, value)?,
            Err(error) => match status(client, clock, &id) {
                Ok(saved) if saved.creation.as_ref() == Some(created) => saved,
                _ => return Err(error),
            },
        };
        if saved.creation.as_ref() != Some(created) {
            return Err(invalid("automatic recorded creation differs"));
        }
        return resolve_and_attach(client, clock, &id, saved);
    }
    if local.not_submitted {
        return parent_result(
            &id,
            call(
                client,
                clock,
                parent_request(
                    &id,
                    AutomaticBootstrapAction::RecordNotSubmitted {
                        attempt: local.attempt,
                    },
                ),
            )?,
        );
    }
    if current.state != BootstrapState::Prepared || local.possible_creation {
        return Err(unknown(reference, current.attempt));
    }
    if submission.witness.endpoint.as_os_str() != namespace.host_endpoint.as_os_str() {
        return Err(invalid(
            "automatic native endpoint differs from frozen namespace",
        ));
    }
    let reserved = call(
        client,
        clock,
        parent_request(
            &id,
            AutomaticBootstrapAction::Reserve {
                attempt: current.attempt,
            },
        ),
    );
    let CommandResult::AutomaticBootstrapReserved(reserved) = (match reserved {
        Ok(value) => value,
        Err(error) => match status(client, clock, &id) {
            Ok(saved) if saved.creation.is_some() => {
                return resolve_and_attach(client, clock, &id, saved);
            }
            Ok(saved) if saved.state != BootstrapState::Prepared => {
                return Err(unknown(reference, saved.attempt));
            }
            _ => return Err(error),
        },
    }) else {
        return Err(invalid("unexpected automatic bootstrap reservation"));
    };
    if reserved.status.identity != id || reserved.status.attempt != current.attempt {
        return Err(invalid("automatic reservation identity differs"));
    }
    let Some(authorization) = reserved.authorization else {
        if reserved.status.creation.is_some() {
            return resolve_and_attach(client, clock, &id, reserved.status);
        }
        return Err(unknown(reference, current.attempt));
    };
    let operation = current
        .attempt
        .operation(&id.plan.payload.keys.compound, "reserve")
        .map_err(invalid)?;
    if authorization.compound != id.plan.payload.keys.compound
        || authorization.attempt != current.attempt
        || authorization.reservation != operation
    {
        return Err(invalid("automatic native authorization differs"));
    }
    let request = CreateTabRequest {
        correlation: HostCallId::new(uuid::Uuid::new_v4().to_string()),
        workspace: id.plan.payload.workspace.clone(),
        cwd: id.plan.payload.cwd.clone(),
        label: id.plan.payload.label.clone(),
        focus: false,
        env: Default::default(),
        expected_witness: submission.witness.clone(),
    };
    local.possible_creation = true;
    local.request = Some(request.clone());
    save_parent(journal, reference, &local).map_err(|_| unknown(reference, current.attempt))?;
    let checked = call(
        client,
        clock,
        parent_request(
            &id,
            AutomaticBootstrapAction::CheckSubmission {
                attempt: current.attempt,
                administrative_revision: authorization.administrative_revision,
            },
        ),
    )?;
    let CommandResult::BootstrapSubmissionChecked(checked) = checked else {
        return Err(invalid("unexpected automatic submission check"));
    };
    if checked.compound != id.plan.payload.keys.compound
        || checked.attempt != current.attempt
        || checked.administrative_revision != authorization.administrative_revision
    {
        return Err(invalid("automatic submission check differs"));
    }
    match native.create_tab(&request, submission.context) {
        CreateTabOutcome::NotSubmitted(_) => {
            local.not_submitted = true;
            save_parent(journal, reference, &local)
                .map_err(|_| unknown(reference, current.attempt))?;
            parent_result(
                &id,
                call(
                    client,
                    clock,
                    parent_request(
                        &id,
                        AutomaticBootstrapAction::RecordNotSubmitted {
                            attempt: current.attempt,
                        },
                    ),
                )?,
            )
        }
        CreateTabOutcome::OutcomeUnknown(_) => {
            let _ = call(
                client,
                clock,
                parent_request(
                    &id,
                    AutomaticBootstrapAction::RecordOutcomeUnknown {
                        attempt: current.attempt,
                    },
                ),
            );
            Err(unknown(reference, current.attempt))
        }
        CreateTabOutcome::Created(created) => {
            if created.validate().is_err()
                || created.correlation != request.correlation
                || created.workspace != request.workspace
                || created.witness != request.expected_witness
            {
                return Err(unknown(reference, current.attempt));
            }
            local.creation = Some(*created);
            save_parent(journal, reference, &local)
                .map_err(|_| unknown(reference, current.attempt))?;
            let created = local.creation.as_ref().unwrap().clone();
            let recorded = call(
                client,
                clock,
                parent_request(
                    &id,
                    AutomaticBootstrapAction::RecordCreated {
                        attempt: current.attempt,
                        evidence: created.clone(),
                    },
                ),
            );
            let saved = match recorded {
                Ok(value) => parent_result(&id, value)?,
                Err(error) => match status(client, clock, &id) {
                    Ok(saved) if saved.creation.as_ref() == Some(&created) => saved,
                    _ => return Err(error),
                },
            };
            if saved.creation.as_ref() != Some(&created) {
                return Err(invalid("automatic recorded creation differs"));
            }
            resolve_and_attach(client, clock, &id, saved)
        }
    }
}

fn effective_launch(
    id: &AutomaticBootstrapIdentity,
    attachment: &AutomaticBootstrapAttachment,
    thread: &ThreadId,
) -> Result<crate::cli::launch::LaunchRequest, RunError> {
    let AutomaticOriginal::Handoff(plan) = &attachment.child.original else {
        return Err(invalid("automatic child lacks frozen launch"));
    };
    let template = plan
        .startup_input
        .as_ref()
        .ok_or_else(|| invalid("automatic child lacks frozen startup input"))?;
    let prompt = crate::cli::handoff::bootstrap_v1::prompt(
        thread.as_str(),
        &plan.context,
        &id.claim.instance,
    );
    let (argv, _) = template.apply(&plan.launch.argv, &prompt)?;
    let native = crate::harness::launch::compose_native_argv(
        id.plan.payload.launch.harness,
        argv.clone(),
        vec![],
    )?;
    crate::ports::NativeLaunchRequest::validate_argv(&native).map_err(invalid)?;
    let mut effective = plan.launch.clone();
    effective.argv = argv;
    Ok(effective)
}
fn retained_report(
    id: &AutomaticBootstrapIdentity,
    attachment: &AutomaticBootstrapAttachment,
    thread: &ThreadId,
    report: &serde_json::Value,
) -> Result<LinkedBootstrapReport, RunError> {
    use sha2::{Digest, Sha256};
    let result = LinkedBootstrapReport {
        thread: thread.clone(),
        recipient: attachment.resolved_seat.clone(),
        pane: attachment.created.root_pane.clone(),
        kind: "launch".into(),
        launch: id.plan.payload.launch.clone(),
        report: report.clone(),
        report_digest: format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(report).map_err(std::io::Error::other)?)
        ),
        terminal: attachment.created.terminal.clone(),
        host_incarnation: attachment.created.host_incarnation.clone(),
    };
    result.validate().map_err(invalid)?;
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resume_to_writer<C: LocalClient + ?Sized, N: CreateTabPort + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    _actor: InvocationActor,
    namespace: &HandoffNamespace,
    client: &C,
    native: &N,
    launcher: &mut dyn HandoffLauncher,
    clock: &dyn Clock,
    submission: BootstrapSubmissionInputs<'_>,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<AutomaticBootstrapResult, RunError> {
    let original = super::load_automatic_original(journal, reference)?;
    let id = super::automatic_bootstrap_identity(&original)?;
    exact_namespace(&id, namespace)?;
    let next = resume_to_attachment(
        journal, reference, namespace, client, native, clock, submission,
    )?;
    if next.state == BootstrapState::Completed {
        return present_completed(journal, reference, &id, next, output, writer);
    }
    if next.state != BootstrapState::Attached {
        return Err(ApiError::host_unavailable(format!(
            "automatic bootstrap {} lacks attached recipient; retry original",
            reference.recovery_ref()
        ))
        .into());
    }
    let _lock = super::super::handoff::lock(journal, reference)?;
    let current = status(client, clock, &id)?;
    if current.state == BootstrapState::Completed {
        return present_completed(journal, reference, &id, current, output, writer);
    }
    let attachment = current
        .attachment
        .as_ref()
        .filter(|_| current.state == BootstrapState::Attached)
        .ok_or_else(|| invalid("automatic bootstrap lacks canonical attachment"))?;
    attachment.validate(&id).map_err(invalid)?;
    let mut progress = load_child(journal, reference, &id, attachment)?;
    let child = &attachment.child;
    let begun = call(
        client,
        clock,
        Command::AutomaticHandoffDelivery(Box::new(child.begin())),
    )?;
    let CommandResult::Handoff(begun) = begun else {
        return Err(invalid("unexpected automatic child begin"));
    };
    if begun.state != HandoffState::Live && begun.state != HandoffState::Completed {
        return Err(invalid("automatic child begin state differs"));
    }
    if progress.thread.is_some() && progress.thread != begun.thread {
        return Err(invalid(
            "automatic child saved thread differs from canonical",
        ));
    }
    if progress.thread.is_none() {
        let ensured = call(
            client,
            clock,
            Command::AutomaticHandoffDelivery(Box::new(child.ensure())),
        )?;
        let thread = automatic_selected_thread(&ensured)
            .map_err(invalid)?
            .clone();
        progress.selection = match ensured {
            CommandResult::DirectThreadEnsured(actual)
                if actual.recipient == attachment.resolved_seat =>
            {
                Some(actual)
            }
            CommandResult::ThreadCreated(_) if id.plan.payload.handoff.policy.is_none() => None,
            _ => return Err(invalid("automatic Ensure lacks actual selection")),
        };
        progress.thread = Some(thread);
        save_child(journal, reference, &progress)?;
    }
    let thread = progress.thread.as_ref().unwrap().clone();
    if begun
        .thread
        .as_ref()
        .is_some_and(|selected| selected != &thread)
    {
        return Err(invalid("automatic child selected thread differs"));
    }
    if progress.message.is_none() {
        let message = call(
            client,
            clock,
            Command::AutomaticHandoffDelivery(Box::new(child.send(thread.clone()))),
        )?;
        if !matches!(message, CommandResult::MessageSent(_)) {
            return Err(invalid("unexpected automatic child send"));
        }
        progress.message = Some(message);
        save_child(journal, reference, &progress)?;
    }
    if progress.launch.is_none() && !progress.possible_start {
        let AutomaticOriginal::Handoff(plan) = &child.original else {
            return Err(invalid("automatic child lacks launch"));
        };
        let effective = effective_launch(&id, attachment, &thread)?;
        let prepared =
            launcher.preflight_saved(&plan.launch, &effective, &attachment.resolved_seat)?;
        if prepared.recipient != attachment.resolved_seat {
            return Err(invalid("automatic launch recipient changed"));
        }
        let native = crate::harness::launch::compose_native_argv(
            id.plan.payload.launch.harness,
            effective.argv.clone(),
            vec![],
        )?;
        if prepared.native_argv != native {
            return Err(invalid("automatic selected native input differs"));
        }
        let result = launcher.launch(&effective, &attachment.resolved_seat, &mut |possible| {
            if possible {
                client.call(
                    parent_request(&id, AutomaticBootstrapAction::CheckStart),
                    &budget(clock),
                )?;
                progress.possible_start = true;
                save_child(journal, reference, &progress)
                    .map_err(|e| ApiError::new(ErrorCode::StoreCorrupt, e.to_string()))
            } else {
                let mut reset = progress.clone();
                reset.possible_start = false;
                save_child(journal, reference, &reset)
                    .map_err(|e| ApiError::new(ErrorCode::StoreCorrupt, e.to_string()))?;
                progress = reset;
                Ok(())
            }
        });
        if let Ok(report) = &result {
            if id.plan.payload.handoff.policy.is_some() {
                progress.launch = Some(super::super::handoff::automatic_summary(&report.report)?);
                // The critical started summary precedes optional diagnostic IO. A failure
                // here or afterward retains possible-start and never permits another launch.
                save_child(journal, reference, &progress)?;
                super::super::handoff::save_automatic_diagnostics(
                    journal,
                    reference,
                    &report.report,
                )?;
            } else {
                progress.launch = Some(report.report.clone());
                save_child(journal, reference, &progress)?;
            }
        }
        result?;
    }
    if progress
        .launch
        .as_ref()
        .is_some_and(|v| v["format"] == "automatic_launch_noncompletion_v1")
        || progress.possible_start
            && progress
                .launch
                .as_ref()
                .is_none_or(|v| v["outcome"] != "started")
    {
        return Err(ApiError::unknown_outcome(format!("automatic bootstrap {} downstream possible start; inspect exact pane {}; no automatic relaunch", reference.recovery_ref(), attachment.created.root_pane.as_str())).into());
    }
    let report = progress
        .launch
        .as_ref()
        .ok_or_else(|| invalid("automatic launch lacks retained report"))?;
    let _completed_child = call(
        client,
        clock,
        Command::AutomaticHandoffDelivery(Box::new(child.complete())),
    )?;
    let retained = retained_report(&id, attachment, &thread, report)?;
    let finished = call(
        client,
        clock,
        parent_request(
            &id,
            AutomaticBootstrapAction::Complete {
                attachment: Box::new(attachment.clone()),
                retained,
            },
        ),
    );
    let finished = match finished {
        Ok(result) => parent_result(&id, result)?,
        Err(error) => match status(client, clock, &id) {
            Ok(done) if done.state == BootstrapState::Completed => done,
            _ => return Err(error),
        },
    };
    present_completed(journal, reference, &id, finished, output, writer)
}

fn read_terminal(journal: &Journal, reference: &IntentRef) -> Result<Option<Terminal>, RunError> {
    let Some(bytes) = super::read_linked_bytes(
        &terminal_path(journal, reference),
        super::MAX_BOOTSTRAP_TERMINAL_BYTES,
    )?
    else {
        return Ok(None);
    };
    let saved: Terminal = super::decode_linked(&bytes, super::MAX_BOOTSTRAP_TERMINAL_BYTES)?;
    if saved.version != 2
        || saved.completed.state != BootstrapState::Completed
        || saved.completed.completed.is_none()
        || saved.completed.thread.is_none()
    {
        return Err(invalid("invalid automatic bootstrap terminal"));
    }
    let original = Journal::decode_bootstrap_origin(reference, saved.original.as_bytes())?;
    let id = super::automatic_bootstrap_identity(&original)?;
    if !matches!(saved.message.as_ref(), Some(CommandResult::MessageSent(_)))
        && (saved.message.is_some() || id.plan.payload.handoff.policy.is_none())
    {
        return Err(invalid("invalid automatic bootstrap terminal message"));
    }
    if let Some(selection) = &saved.selection {
        selection.validate().map_err(invalid)?;
        if id.plan.payload.handoff.policy.is_none()
            || Some(&selection.thread) != saved.completed.thread.as_ref()
            || saved
                .completed
                .attachment
                .as_ref()
                .is_none_or(|attachment| selection.recipient != attachment.resolved_seat)
        {
            return Err(invalid("automatic terminal selection differs"));
        }
    }
    if saved.completed.identity != id {
        return Err(invalid("automatic terminal original differs"));
    }
    match journal.snapshot_bootstrap_origin(reference) {
        Ok(bytes) if bytes != saved.original.as_bytes() => {
            return Err(invalid("automatic terminal origin contradiction"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(Some(saved))
}
pub(super) fn load_original(
    journal: &Journal,
    reference: &IntentRef,
) -> Result<crate::cli::journal::PendingIntent, RunError> {
    if let Some(saved) = read_terminal(journal, reference)? {
        return Ok(Journal::decode_bootstrap_origin(
            reference,
            saved.original.as_bytes(),
        )?);
    }
    let bytes = journal.snapshot_bootstrap_origin(reference)?;
    let pending = Journal::decode_bootstrap_origin(reference, &bytes)?;
    super::automatic_bootstrap_identity(&pending)?;
    Ok(pending)
}
pub(super) fn resolve_recovery_ref(journal: &Journal, value: &str) -> Result<IntentRef, RunError> {
    let ordinal: u64 = value
        .strip_prefix("local:")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0 && value == format!("local:{v}"))
        .ok_or_else(|| invalid("invalid automatic bootstrap reference"))?;
    let mut found = None;
    for entry in std::fs::read_dir(journal.root())? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if let Some(operation) = name
            .strip_prefix(&format!("automatic-bootstrap-{ordinal:020}-"))
            .and_then(|v| v.strip_suffix(".terminal"))
            .or_else(|| {
                name.strip_prefix(&format!("{ordinal:020}-"))
                    .and_then(|v| v.strip_suffix(".intent"))
            })
        {
            let reference = IntentRef {
                ordinal,
                operation: crate::protocol::ids::OperationId::parse(operation.to_owned())
                    .map_err(invalid)?,
            };
            load_original(journal, &reference)?;
            if found.as_ref().is_some_and(|prior| prior != &reference) {
                return Err(invalid("ambiguous automatic bootstrap reference"));
            }
            found = Some(reference);
        }
    }
    found.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "automatic bootstrap reference not found",
        )
        .into()
    })
}
pub(super) fn try_completed<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    namespace: &HandoffNamespace,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<bool, RunError> {
    let original = load_original(journal, reference)?;
    if crate::cli::journal::classify_original_actor(&original.header.scope, &original.semantic)?
        != crate::cli::journal::OriginalActor::Agent
    {
        return Err(invalid("automatic bootstrap needs original agent"));
    }
    let id = super::automatic_bootstrap_identity(&original)?;
    exact_namespace(&id, namespace)?;
    let _lock = super::super::handoff::lock(journal, reference)?;
    let retained = read_terminal(journal, reference)?;
    let current = match status(client, clock, &id) {
        Ok(current) => current,
        Err(RunError::Api(e)) if e.code == ErrorCode::Conflict && retained.is_none() => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    if current.state != BootstrapState::Completed {
        if retained.is_some() {
            return Err(invalid("automatic terminal lacks canonical completion"));
        }
        if current.state != BootstrapState::Attached {
            return Ok(false);
        }
        let attachment = current
            .attachment
            .as_ref()
            .ok_or_else(|| invalid("automatic bootstrap lacks canonical attachment"))?;
        attachment.validate(&id).map_err(invalid)?;
        let mut child_status = attachment.child.clone();
        child_status.action = AutomaticDeliveryAction::Status;
        let child = match call(
            client,
            clock,
            Command::AutomaticHandoffDelivery(Box::new(child_status)),
        ) {
            Ok(CommandResult::Handoff(child)) => child,
            Ok(_) => return Err(invalid("unexpected automatic child status")),
            Err(RunError::Api(e))
                if matches!(e.code, ErrorCode::Conflict | ErrorCode::NotFound) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        if child.state != HandoffState::Completed {
            return Ok(false);
        }
        let progress = load_child(journal, reference, &id, attachment)?;
        let thread = progress
            .thread
            .as_ref()
            .filter(|thread| child.thread.as_ref() == Some(*thread))
            .filter(|thread| current.thread.as_ref() == Some(*thread))
            .ok_or_else(|| invalid("automatic child retained thread differs"))?;
        if !matches!(progress.message, Some(CommandResult::MessageSent(_))) {
            return Err(invalid("automatic child lacks retained sent message"));
        }
        let report = progress
            .launch
            .as_ref()
            .filter(|report| progress.possible_start && report["outcome"] == "started")
            .ok_or_else(|| invalid("automatic child lacks successful retained launch"))?;
        let retained = retained_report(&id, attachment, thread, report)?;
        let finished = parent_result(
            &id,
            call(
                client,
                clock,
                parent_request(
                    &id,
                    AutomaticBootstrapAction::Complete {
                        attachment: Box::new(attachment.clone()),
                        retained,
                    },
                ),
            )?,
        )?;
        present_completed(journal, reference, &id, finished, output, writer)?;
        return Ok(true);
    }
    present_completed(journal, reference, &id, current, output, writer)?;
    Ok(true)
}
pub(super) fn write_pending<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    namespace: &HandoffNamespace,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let original = load_original(journal, reference)?;
    let id = super::automatic_bootstrap_identity(&original)?;
    exact_namespace(&id, namespace)?;
    let current = status(client, clock, &id).ok();
    if current
        .as_ref()
        .is_some_and(|v| v.state == BootstrapState::Completed)
    {
        return Ok(());
    }
    let child = current
        .as_ref()
        .and_then(|v| v.attachment.as_ref())
        .map(|attachment| load_child(journal, reference, &id, attachment))
        .transpose()?;
    let possible_start = child.as_ref().is_some_and(|progress| {
        progress.possible_start
            && progress
                .launch
                .as_ref()
                .is_none_or(|v| v["outcome"] != "started")
    });
    let started = child.as_ref().is_some_and(|progress| {
        progress
            .launch
            .as_ref()
            .is_some_and(|v| v["outcome"] == "started")
    });
    let unknown_creation = current
        .as_ref()
        .is_some_and(|v| v.state == BootstrapState::PossibleCreation && v.creation.is_none());
    let mut retry = crate::cli::hook::cli_prefix(&output.context);
    retry.extend(["retry".into(), reference.recovery_ref()]);
    let report = serde_json::json!({
        "bootstrap": {
            "failed": true,
            "outcome": if unknown_creation { "creation_unknown" } else if possible_start { "possible_start" }
                else if started { "completion_pending" } else { "pending" },
            "recovery_ref": reference.recovery_ref(),
            "namespace": namespace,
            "bootstrap_compound": id.plan.payload.keys.compound,
            "state": current.as_ref().map(|v| v.state),
            "attempt": current.as_ref().map(|v| v.attempt),
            "status_is_last_observed": current.is_some(),
            "status_unknown": current.is_none(),
            "creation": current.as_ref().and_then(|v| v.creation.as_ref()),
            "seat": current.as_ref().and_then(|v| v.attachment.as_ref()).map(|a| &a.resolved_seat),
            "pane": current.as_ref().and_then(|v| v.creation.as_ref()).map(|c| &c.root_pane),
            "thread": current.as_ref().and_then(|v| v.thread.as_ref()),
            "message": child.as_ref().and_then(|v| v.message.as_ref()),
            "possible_start": possible_start,
            "retry_argv": retry,
            "guidance": super::super::handoff::AUTOMATIC_BOOTSTRAP_GUIDANCE
        }
    });
    let bytes = if output.format == OutputFormat::Json {
        format!("{report}\n").into_bytes()
    } else {
        crate::cli::setup::render_text(&report).into_bytes()
    };
    if bytes.len() > 1024 * 1024 {
        return Err(invalid("oversized automatic pending report"));
    }
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}
fn save_terminal(
    journal: &Journal,
    reference: &IntentRef,
    terminal: &Terminal,
) -> Result<(), RunError> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let bytes = serde_json::to_vec(terminal).map_err(std::io::Error::other)?;
    if bytes.len() > super::MAX_BOOTSTRAP_TERMINAL_BYTES {
        return Err(invalid("oversized automatic bootstrap terminal"));
    }
    let path = terminal_path(journal, reference);
    let temp = journal.root().join(format!(
        ".automatic-bootstrap-terminal-{}",
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::hard_link(&temp, &path)?;
        std::fs::File::open(journal.root())?.sync_all()
    })();
    let cleanup = std::fs::remove_file(&temp);
    result?;
    cleanup?;
    std::fs::File::open(journal.root())?.sync_all()?;
    Ok(())
}
fn present_completed<W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    id: &AutomaticBootstrapIdentity,
    completed: AutomaticBootstrapResult,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<AutomaticBootstrapResult, RunError> {
    if completed.identity != *id
        || completed.state != BootstrapState::Completed
        || completed.completed.is_none()
        || completed.thread.is_none()
    {
        return Err(invalid("automatic canonical completion differs"));
    }
    if id.plan.payload.handoff.policy.is_some() {
        let attachment = completed
            .attachment
            .as_ref()
            .ok_or_else(|| invalid("automatic completion lacks attachment"))?;
        let retained = completed.completed.as_ref().unwrap();
        attachment.validate(id).map_err(invalid)?;
        AutomaticBootstrapMutation {
            identity: id.clone(),
            action: AutomaticBootstrapAction::Complete {
                attachment: Box::new(attachment.clone()),
                retained: retained.clone(),
            },
        }
        .validate()
        .map_err(invalid)?;
        let full_digest = retained.report["full_report_digest"]
            .as_str()
            .ok_or_else(|| invalid("automatic completion lacks summary digest"))?;
        if automatic_launch_summary(&retained.report, full_digest).map_err(invalid)?
            != retained.report
            || completed.thread.as_ref() != Some(&retained.thread)
        {
            return Err(invalid("automatic canonical summary differs"));
        }
    }
    let terminal = read_terminal(journal, reference)?;
    if terminal
        .as_ref()
        .is_some_and(|saved| saved.completed != completed)
    {
        return Err(invalid(
            "automatic retained and canonical completion differ",
        ));
    }
    let message = if let Some(saved) = &terminal {
        saved.message.clone()
    } else if let Some(attachment) = &completed.attachment {
        load_child(journal, reference, id, attachment)?.message
    } else {
        None
    };
    if !matches!(message.as_ref(), Some(CommandResult::MessageSent(_)))
        && (message.is_some() || id.plan.payload.handoff.policy.is_none())
    {
        return Err(invalid("automatic completion lacks retained sent message"));
    }
    let selection = if let Some(saved) = &terminal {
        saved.selection.clone()
    } else if let Some(attachment) = &completed.attachment {
        load_child(journal, reference, id, attachment)?.selection
    } else {
        None
    };
    if terminal.is_none() {
        let original = String::from_utf8(journal.snapshot_bootstrap_origin(reference)?)
            .map_err(std::io::Error::other)?;
        save_terminal(
            journal,
            reference,
            &Terminal {
                selection: selection.clone(),
                version: 2,
                original,
                completed: completed.clone(),
                message: message.clone(),
            },
        )?;
    }
    let mut report = serde_json::json!({
        "bootstrap": { "failed": false, "outcome": "started", "state": "completed",
            "recovery_ref": reference.recovery_ref(), "thread": completed.thread,
            "seat": completed.attachment.as_ref().map(|a| &a.resolved_seat),
            "pane": completed.attachment.as_ref().map(|a| &a.created.root_pane),
            "message": message, "launch": completed.completed.as_ref().map(|r| &r.report),
        }
    });
    if let Some(selection) = selection {
        report["bootstrap"]["selection"] = selection.presentation();
    }
    if message.is_none() {
        report["bootstrap"]["message_status"] = serde_json::json!("unavailable");
    }
    if let Some(retained) = &completed.completed
        && retained.report["format"] == "automatic_launch_summary_v1"
    {
        report["bootstrap"]["local_diagnostics"] =
            super::super::handoff::automatic_diagnostics_marker(
                journal,
                reference,
                &retained.report,
            );
        report["bootstrap"]["launch"] =
            super::super::handoff::automatic_launch_presentation(&retained.report);
    }
    let bytes = if output.format == OutputFormat::Json {
        format!("{report}\n").into_bytes()
    } else {
        super::super::setup::render_text(&report).into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    match journal.complete(reference) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && terminal.is_some() => {}
        Err(error) => return Err(error.into()),
    }
    Ok(completed)
}
