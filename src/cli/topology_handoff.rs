//! Immutable new-tab preparation. Public execution remains fenced until activation.
use super::{RunError, panes::PaneSelector};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub new_tab: Option<String>,
    pub existing: bool,
    pub selector: PaneSelector,
    pub seat: Option<String>,
    pub cwd: Option<std::path::PathBuf>,
    pub thread: Option<String>,
    pub thread_name: Option<String>,
    pub topic: Option<String>,
    pub goal: Option<String>,
    pub body: String,
    pub kind: Option<String>,
    pub binary: Option<String>,
    pub name: Option<String>,
    pub argv: Vec<String>,
}

pub struct Preparation<'a, C: crate::ports::LocalClient + ?Sized> {
    pub claim: &'a crate::protocol::authority::CallerClaim,
    pub namespace: &'a crate::protocol::handoff::HandoffNamespace,
    pub topology: &'a crate::host::observation::HostTopology,
    pub invocation_cwd: &'a std::path::Path,
    /// Captured once from the matching launch options variable by the caller.
    pub options: Option<std::ffi::OsString>,
    pub client: &'a C,
    pub clock: &'a dyn crate::protocol::time::Clock,
}
/// Resolve read-only selections once; return the full validated original identity.
/// Namespace input must be the selected runtime namespace. Canonical execution
/// additionally compares it with the daemon's own paths before any effect.
pub fn prepare<C: crate::ports::LocalClient + ?Sized>(
    request: &Request,
    inputs: Preparation<'_, C>,
) -> Result<crate::protocol::handoff::BootstrapIdentity, RunError> {
    use crate::protocol::{
        commands::{Command, ResolveThreadQuery},
        handoff::*,
        ids::{HostTargetId, OperationId},
        results::CommandResult,
    };
    let fail = super::invalid_request;
    let label = request
        .new_tab
        .as_ref()
        .ok_or_else(|| fail("bootstrap requires --new-tab"))?;
    if request.existing
        || request.seat.is_some()
        || request.selector.tab.is_some()
        || request.selector.pane.is_some()
    {
        return Err(fail("conflicting bootstrap target"));
    }
    if request.thread.is_some()
        && (request.thread_name.is_some() || request.topic.is_some() || request.goal.is_some())
    {
        return Err(fail("new channel fields conflict with existing thread"));
    }
    let kind = request
        .kind
        .as_deref()
        .ok_or_else(|| fail("bootstrap requires --kind"))?;
    let harness = match kind {
        "codex" => crate::harness::context::Harness::Codex,
        "claude" => crate::harness::context::Harness::Claude,
        _ => return Err(fail("invalid bootstrap harness")),
    };
    let cwd = request.cwd.as_deref().unwrap_or(inputs.invocation_cwd);
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(fail("--cwd must be an absolute existing directory"));
    }
    let cwd = cwd.canonicalize()?;
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        inputs.namespace.state_dir.clone(),
        inputs.namespace.host_endpoint.clone(),
        None,
    )?;
    let namespace = HandoffNamespace {
        instance: inputs.namespace.instance.clone(),
        state_dir: runtime.state_dir,
        host_endpoint: runtime.host_endpoint,
    };
    let workspace = if let Some(value) = &request.selector.space {
        if let Some(exact) = inputs
            .topology
            .spaces
            .iter()
            .find(|space| &space.id == value)
        {
            &exact.id
        } else {
            let mut found = inputs
                .topology
                .spaces
                .iter()
                .filter(|space| space.label.as_ref() == Some(value));
            let selected = found
                .next()
                .ok_or_else(|| fail("workspace is absent from live topology"))?;
            if found.next().is_some() {
                return Err(fail("ambiguous workspace; use its exact ID"));
            }
            &selected.id
        }
    } else {
        let caller = inputs
            .topology
            .panes
            .iter()
            .find(|pane| pane.target == inputs.claim.target)
            .ok_or_else(|| fail("caller pane is absent from live topology"))?;
        &inputs
            .topology
            .spaces
            .iter()
            .find(|space| space.id == caller.space)
            .ok_or_else(|| fail("caller workspace is absent from live topology"))?
            .id
    };
    let channel = if let Some(selector) = &request.thread {
        let result = inputs.client.call(
            Command::ResolveThread(ResolveThreadQuery {
                selector: selector.clone(),
                caller: Some(inputs.claim.seat.clone()),
                caller_target: Some(inputs.claim.target.clone()),
            }),
            &super::cooperative_budget(inputs.clock),
        )?;
        let CommandResult::ThreadResolved(thread) = result else {
            return Err(fail("unexpected thread resolution result"));
        };
        HandoffChannel::Existing { thread }
    } else {
        let topic = request
            .topic
            .clone()
            .unwrap_or_else(|| format!("Handoff to {label}"));
        HandoffChannel::New {
            name: request.thread_name.clone(),
            goal: request.goal.clone().unwrap_or_else(|| topic.clone()),
            topic,
        }
    };
    let launch = super::launch::LaunchRequest {
        target: inputs.claim.target.clone(),
        harness,
        harness_binary: request.binary.clone(),
        name: request.name.clone(),
        pane_label: None,
        argv: request.argv.clone(),
    }
    .with_configured_options(inputs.options)?;
    let compound = OperationId::new(uuid::Uuid::new_v4().to_string());
    let child = |phase: &str| {
        use sha2::{Digest, Sha256};
        OperationId::new(format!(
            "bootstrap-{:x}",
            Sha256::digest(format!("{}\0{phase}", compound.as_str()).as_bytes())
        ))
    };
    let payload = BootstrapPayload {
        handoff: HandoffPayload {
            namespace,
            keys: HandoffKeys {
                compound: compound.clone(),
                begin: child("begin"),
                create: child("create"),
                invite: child("invite"),
                send: child("send"),
                complete: child("complete"),
            },
            channel,
            body: request.body.clone(),
        },
        workspace: HostTargetId::parse(workspace).map_err(fail)?,
        cwd,
        label: label.clone(),
        focus: false,
        env: Default::default(),
        launch: BootstrapLaunch {
            harness: match harness {
                crate::harness::context::Harness::Codex => {
                    crate::protocol::authority::Harness::Codex
                }
                _ => crate::protocol::authority::Harness::Claude,
            },
            binary: launch.harness_binary,
            name: launch
                .name
                .as_deref()
                .map(|name| {
                    crate::ports::sanitize_agent_name(name)
                        .ok_or_else(|| fail("invalid agent --name"))
                })
                .transpose()?,
            argv: launch.argv,
        },
        handoff_key: child("handoff"),
        resolve_key: child("resolve"),
        attach_key: child("attach"),
        linked_complete_key: child("linked-complete"),
    };
    let mut identity = BootstrapIdentity {
        compound: compound.clone(),
        scope: super::journal::IntentScope::Cooperative {
            instance: inputs.claim.instance.clone(),
            seat: inputs.claim.seat.clone(),
        },
        claim: inputs.claim.clone(),
        digest: String::new(),
        payload,
    };
    identity.digest = identity.semantic_digest().map_err(fail)?;
    identity.validate().map_err(fail)?;
    Ok(identity)
}

/// Atomic journal publication uses the established allocator/lock/fsync contract.
/// Its local reference is independent of the payload's canonical compound key.
/// Failed validation publishes nothing. Retry must load this saved semantic,
/// never call prepare again for that operation.
pub fn publish(
    journal: &super::journal::Journal,
    identity: &crate::protocol::handoff::BootstrapIdentity,
    created_at_millis: i64,
) -> Result<super::journal::IntentRef, RunError> {
    use super::journal::{BootstrapPlan, SemanticMutation};
    identity.validate().map_err(super::invalid_request)?;
    if !identity.payload.cwd.is_dir() {
        return Err(super::invalid_request(
            "bootstrap cwd disappeared before publication",
        ));
    }
    let semantic = SemanticMutation::freeze(
        SemanticMutation::HandoffBootstrap(Box::new(BootstrapPlan {
            version: 1,
            payload: identity.payload.clone(),
        })),
        identity.claim.clone(),
    )?;
    let reference = journal.record(identity.scope.clone(), semantic.clone(), created_at_millis)?;
    let saved = journal.load(&reference)?;
    if saved.header.scope != identity.scope
        || saved.header.semantic_digest != identity.digest
        || saved.semantic != semantic
    {
        return Err(super::invalid_request(
            "published bootstrap identity differs from prepared intent",
        ));
    }
    Ok(reference)
}
/// Expected native endpoint witness captured by the actual qualified transport.
/// The caller must admit the original actor before invoking this internal seam.
pub struct BootstrapSubmissionInputs<'a> {
    pub witness: &'a crate::host::continuity::LocalEndpointWitness,
    pub context: &'a crate::ports::HostCallContext,
}

/// Internal live coordinator through canonical exact attachment. Public routes
/// remain Unsupported; activation owns actor admission and the fresh daemon guard.
#[allow(clippy::too_many_arguments)]
pub fn resume_to_attachment<
    C: crate::ports::LocalClient + ?Sized,
    N: crate::ports::CreateTabPort + ?Sized,
>(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
    namespace: &crate::protocol::handoff::HandoffNamespace,
    client: &C,
    native: &N,
    clock: &dyn crate::protocol::time::Clock,
    submission: BootstrapSubmissionInputs<'_>,
) -> Result<crate::protocol::handoff::BootstrapResult, RunError> {
    use crate::{
        ports::*,
        protocol::{commands::Command, handoff::*, ids::HostCallId, results::CommandResult},
    };
    let pending = journal.load(reference)?;
    let super::journal::SemanticMutation::Frozen { claim, mutation } = pending.semantic else {
        return Err(super::invalid_request(
            "bootstrap needs its original frozen caller",
        ));
    };
    let super::journal::SemanticMutation::HandoffBootstrap(plan) = *mutation else {
        return Err(super::invalid_request("not a bootstrap reference"));
    };
    let identity = BootstrapIdentity {
        compound: plan.payload.handoff.keys.compound.clone(),
        scope: pending.header.scope,
        claim,
        digest: pending.header.semantic_digest,
        payload: plan.payload,
    };
    crate::store::topology_handoff::encode_identity(namespace, &identity)?;
    match client.call(Command::Capabilities, &super::cooperative_budget(clock)) {
        Ok(CommandResult::Capabilities(c))
            if c.capabilities.iter().any(|name| {
                name == crate::protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1
            }) => {}
        _ => {
            return Err(super::unsupported(
                "daemon lacks guarded bootstrap resolution capability",
            ));
        }
    }
    let _lock = super::handoff::lock(journal, reference)?;
    let mut progress = load_bootstrap_progress(journal, reference, &identity)?;
    let call = |command| {
        client
            .call(command, &super::cooperative_budget(clock))
            .map_err(RunError::from)
    };
    let status = || -> Result<BootstrapResult, RunError> {
        bootstrap_result(
            &identity,
            call(Command::BootstrapStatus(Box::new(BootstrapStatus {
                identity: identity.clone(),
            })))?,
        )
    };
    let begin = call(Command::BeginBootstrap(Box::new(BeginBootstrap {
        operation: identity.payload.handoff.keys.begin.clone(),
        identity: identity.clone(),
    })));
    let mut current = match begin {
        Ok(v) => bootstrap_result(&identity, v)?,
        Err(error) => match status() {
            Ok(v) => v,
            Err(_) => return Err(error),
        },
    };
    if current.state == BootstrapState::Completed {
        return Ok(current);
    }
    if current.state == BootstrapState::Cancelled {
        return Err(super::invalid_request("bootstrap is cancelled"));
    }
    if let Some(local) = &progress {
        if local.attempt > current.attempt
            || (local.attempt < current.attempt && local.creation.is_some())
        {
            return Err(super::invalid_request(
                "bootstrap local evidence contradicts canonical attempt",
            ));
        }
        if local.attempt < current.attempt {
            progress = None;
        }
    }
    let mut local = progress.unwrap_or_else(|| BootstrapProgress {
        version: 1,
        identity: identity.clone(),
        attempt: current.attempt,
        possible_creation: false,
        request: None,
        creation: None,
        not_submitted: false,
    });
    if let Some(created) = &current.creation {
        if local
            .creation
            .as_ref()
            .is_some_and(|saved| saved != created)
        {
            return Err(super::invalid_request(
                "local and canonical creation evidence differ",
            ));
        }
        local.possible_creation = true;
        local.creation = Some(created.clone());
        save_bootstrap_progress(journal, reference, &local)?;
        return resolve_and_attach(&identity, &current, &call, &status);
    }
    if local.not_submitted {
        return close_not_submitted(&identity, local.attempt, &call);
    }
    if let Some(evidence) = &local.creation {
        if local.request.is_none() {
            return Err(creation_unknown(reference, &identity, local.attempt));
        }
        current = record_creation(&identity, local.attempt, evidence, &call, &status)?;
        if current.creation.as_ref() != Some(evidence) {
            return Err(super::invalid_request("canonical creation result differs"));
        }
        return resolve_and_attach(&identity, &current, &call, &status);
    }
    if current.state != BootstrapState::Prepared || local.possible_creation {
        return Err(creation_unknown(reference, &identity, current.attempt));
    }
    if submission.witness.endpoint.as_os_str() != namespace.host_endpoint.as_os_str() {
        return Err(super::invalid_request(
            "native submission endpoint differs from frozen namespace",
        ));
    }
    let operation = current
        .attempt
        .operation(&identity.compound, "reserve")
        .map_err(super::invalid_request)?;
    let reserved = call(Command::ReserveBootstrapAttempt(Box::new(
        ReserveBootstrapAttempt {
            identity: identity.clone(),
            operation: operation.clone(),
            expected_attempt: current.attempt,
        },
    )));
    let authorization = match reserved {
        Ok(CommandResult::BootstrapReserved(result)) => match *result {
            ReserveBootstrapResult::Authorized { authorization } => authorization,
            ReserveBootstrapResult::Replay { status: replay } => {
                let replay = bootstrap_result(&identity, CommandResult::Bootstrap(replay))?;
                if replay.creation.is_some() {
                    return resolve_and_attach(&identity, &replay, &call, &status);
                }
                return Err(creation_unknown(reference, &identity, replay.attempt));
            }
        },
        Ok(_) => {
            return Err(super::invalid_request(
                "unexpected bootstrap reservation result",
            ));
        }
        Err(error) => {
            if let Ok(saved) = status() {
                if saved.creation.is_some() {
                    return resolve_and_attach(&identity, &saved, &call, &status);
                }
                if saved.state != BootstrapState::Prepared {
                    return Err(creation_unknown(reference, &identity, saved.attempt));
                }
            }
            return Err(error);
        }
    };
    if authorization.compound != identity.compound
        || authorization.attempt != current.attempt
        || authorization.reservation != operation
    {
        return Err(super::invalid_request(
            "bootstrap submission authorization differs",
        ));
    }
    let request = CreateTabRequest {
        correlation: HostCallId::new(uuid::Uuid::new_v4().to_string()),
        workspace: identity.payload.workspace.clone(),
        cwd: identity.payload.cwd.clone(),
        label: identity.payload.label.clone(),
        focus: identity.payload.focus,
        env: identity.payload.env.clone(),
        expected_witness: submission.witness.clone(),
    };
    local.possible_creation = true;
    local.request = Some(request.clone());
    save_bootstrap_progress(journal, reference, &local)
        .map_err(|_| creation_unknown(reference, &identity, current.attempt))?;
    let checked = call(Command::CheckBootstrapSubmission(Box::new(
        CheckBootstrapSubmission {
            identity: identity.clone(),
            operation: current
                .attempt
                .operation(&identity.compound, "check")
                .map_err(super::invalid_request)?,
            expected_attempt: current.attempt,
            expected_administrative_revision: authorization.administrative_revision,
        },
    )))?;
    let CommandResult::BootstrapSubmissionChecked(checked) = checked else {
        return Err(super::invalid_request(
            "unexpected bootstrap submission check",
        ));
    };
    if checked.compound != identity.compound
        || checked.attempt != current.attempt
        || checked.administrative_revision != authorization.administrative_revision
    {
        return Err(super::invalid_request(
            "fresh bootstrap submission check differs",
        ));
    }
    match native.create_tab(&request, submission.context) {
        CreateTabOutcome::NotSubmitted(_) => {
            local.not_submitted = true;
            save_bootstrap_progress(journal, reference, &local)
                .map_err(|_| creation_unknown(reference, &identity, current.attempt))?;
            close_not_submitted(&identity, current.attempt, &call)
        }
        CreateTabOutcome::OutcomeUnknown(_) => {
            Err(creation_unknown(reference, &identity, current.attempt))
        }
        CreateTabOutcome::Created(created) => {
            if created.validate().is_err()
                || created.correlation != request.correlation
                || created.workspace != request.workspace
                || created.witness != request.expected_witness
            {
                return Err(creation_unknown(reference, &identity, current.attempt));
            }
            local.creation = Some(*created);
            save_bootstrap_progress(journal, reference, &local)
                .map_err(|_| creation_unknown(reference, &identity, current.attempt))?;
            let evidence = local.creation.as_ref().unwrap();
            current = record_creation(&identity, current.attempt, evidence, &call, &status)?;
            if current.creation.as_ref() != Some(evidence) {
                return Err(super::invalid_request("canonical creation result differs"));
            }
            resolve_and_attach(&identity, &current, &call, &status)
        }
    }
}

/// Derive the legacy immutable plan from the original frozen options and the
/// exact canonical recipient. No downstream journal/effect is produced here.
fn downstream_plan(
    identity: &crate::protocol::handoff::BootstrapIdentity,
    created: &crate::ports::CreatedTab,
    recipient: &crate::protocol::ids::SeatId,
) -> Result<super::handoff::HandoffPlan, RunError> {
    use crate::{harness::context::Harness, protocol::handoff::HandoffChannel};
    let payload = &identity.payload;
    let (thread, thread_name, topic, goal) = match &payload.handoff.channel {
        HandoffChannel::Existing { thread } => (Some(thread.clone()), None, None, None),
        HandoffChannel::New { name, topic, goal } => {
            (None, name.clone(), Some(topic.clone()), Some(goal.clone()))
        }
    };
    let harness = match payload.launch.harness {
        crate::protocol::authority::Harness::Codex => Harness::Codex,
        crate::protocol::authority::Harness::Claude => Harness::Claude,
        crate::protocol::authority::Harness::Human => {
            return Err(super::invalid_request(
                "bootstrap cannot launch human harness",
            ));
        }
    };
    let plan = super::handoff::HandoffPlan {
        request: super::handoff::HandoffRequest {
            thread,
            thread_name,
            topic,
            goal,
            body: payload.handoff.body.clone(),
            launch: super::launch::LaunchRequest {
                target: created.root_pane.clone(),
                harness,
                harness_binary: payload.launch.binary.clone(),
                argv: payload.launch.argv.clone(),
                name: payload.launch.name.clone(),
                pane_label: Some(payload.label.clone()),
            },
        },
        context: crate::protocol::output::ContinuationContext {
            state_dir: Some(
                payload
                    .handoff
                    .namespace
                    .state_dir
                    .to_str()
                    .ok_or_else(|| super::invalid_request("bootstrap state path is not UTF-8"))?
                    .into(),
            ),
            host: Some(
                payload
                    .handoff
                    .namespace
                    .host_endpoint
                    .to_str()
                    .ok_or_else(|| super::invalid_request("bootstrap endpoint is not UTF-8"))?
                    .into(),
            ),
        },
        recipient: recipient.clone(),
        create_key: payload.handoff.keys.create.clone(),
        invite_key: payload.handoff.keys.invite.clone(),
        send_key: payload.handoff.keys.send.clone(),
    };
    plan.validate()?;
    Ok(plan)
}
fn downstream_identity(
    identity: &crate::protocol::handoff::BootstrapIdentity,
    plan: &super::handoff::HandoffPlan,
) -> Result<crate::protocol::handoff::HandoffIdentity, RunError> {
    use sha2::{Digest, Sha256};
    let semantic = super::journal::SemanticMutation::freeze(
        super::journal::SemanticMutation::Handoff(Box::new(plan.clone())),
        identity.claim.clone(),
    )?;
    Ok(crate::protocol::handoff::HandoffIdentity {
        compound: identity.payload.handoff_key.clone(),
        digest: format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&semantic).map_err(std::io::Error::other)?)
        ),
        claim: identity.claim.clone(),
        thread: plan.request.thread.clone(),
        recipient: plan.recipient.clone(),
        create_key: plan.create_key.clone(),
        invite_key: plan.invite_key.clone(),
        send_key: plan.send_key.clone(),
    })
}
/// The downstream consumer must use this exact plan, retaining the saved child
/// compound and digest. It owns its own staged work, launch and terminal handling.
pub fn attached_handoff_plan(
    identity: &crate::protocol::handoff::BootstrapIdentity,
    attachment: &crate::protocol::handoff::BootstrapAttachment,
) -> Result<super::handoff::HandoffPlan, RunError> {
    identity.validate().map_err(super::invalid_request)?;
    attachment
        .validate(identity)
        .map_err(super::invalid_request)?;
    let plan = downstream_plan(identity, &attachment.created, &attachment.resolved_seat)?;
    if downstream_identity(identity, &plan)? != attachment.handoff {
        return Err(super::invalid_request(
            "bootstrap downstream immutable identity differs",
        ));
    }
    Ok(plan)
}
fn resolve_and_attach(
    identity: &crate::protocol::handoff::BootstrapIdentity,
    current: &crate::protocol::handoff::BootstrapResult,
    call: &impl Fn(
        crate::protocol::commands::Command,
    ) -> Result<crate::protocol::results::CommandResult, RunError>,
    status: &impl Fn() -> Result<crate::protocol::handoff::BootstrapResult, RunError>,
) -> Result<crate::protocol::handoff::BootstrapResult, RunError> {
    use crate::protocol::{commands::Command, handoff::*, results::CommandResult};
    let created = current
        .creation
        .as_ref()
        .ok_or_else(|| super::invalid_request("bootstrap canonical creation missing"))?;
    let recipient = match call(Command::ResolveBootstrapSeat(Box::new(
        ResolveBootstrapSeat {
            identity: identity.clone(),
            expected_attempt: current.attempt,
            operation: identity.payload.resolve_key.clone(),
        },
    )))? {
        CommandResult::SeatResolved(seat) => seat,
        _ => {
            return Err(super::invalid_request(
                "unexpected bootstrap resolution result",
            ));
        }
    };
    let plan = downstream_plan(identity, created, &recipient)?;
    let attachment = BootstrapAttachment {
        attempt: current.attempt,
        created: created.clone(),
        resolve_operation: identity.payload.resolve_key.clone(),
        resolved_seat: recipient,
        handoff: downstream_identity(identity, &plan)?,
    };
    attachment
        .validate(identity)
        .map_err(super::invalid_request)?;
    if current
        .attachment
        .as_ref()
        .is_some_and(|saved| saved != &attachment)
    {
        return Err(super::invalid_request(
            "canonical bootstrap attachment differs",
        ));
    }
    let response = call(Command::AttachBootstrapHandoff(Box::new(
        AttachBootstrapHandoff {
            identity: identity.clone(),
            operation: identity.payload.attach_key.clone(),
            attachment: attachment.clone(),
        },
    )));
    let result = match response {
        Ok(v) => bootstrap_result(identity, v)?,
        Err(error) => match status() {
            Ok(saved)
                if saved.attempt == current.attempt
                    && saved.attachment.as_ref() == Some(&attachment) =>
            {
                saved
            }
            _ => return Err(error),
        },
    };
    if result.attempt != current.attempt
        || result.attachment.as_ref() != Some(&attachment)
        || !matches!(
            result.state,
            BootstrapState::Attached | BootstrapState::Completed
        )
    {
        return Err(super::invalid_request(
            "bootstrap attachment result differs",
        ));
    }
    Ok(result)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BootstrapProgress {
    pub version: u32,
    pub identity: crate::protocol::handoff::BootstrapIdentity,
    pub attempt: crate::protocol::handoff::BootstrapAttempt,
    pub possible_creation: bool,
    pub request: Option<crate::ports::CreateTabRequest>,
    pub creation: Option<crate::ports::CreatedTab>,
    pub not_submitted: bool,
}
const MAX_BOOTSTRAP_PROGRESS: usize = 128 * 1024;
fn save_bootstrap_progress(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
    progress: &BootstrapProgress,
) -> Result<(), RunError> {
    if serde_json::to_vec(progress)
        .map_err(std::io::Error::other)?
        .len()
        > MAX_BOOTSTRAP_PROGRESS
    {
        return Err(super::invalid_request("oversized bootstrap progress"));
    }
    super::handoff::save_progress(journal, reference, progress)?;
    Ok(())
}
fn load_bootstrap_progress(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
    identity: &crate::protocol::handoff::BootstrapIdentity,
) -> Result<Option<BootstrapProgress>, RunError> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(super::handoff::progress_path(journal, reference))
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !file.metadata()?.is_file() {
        return Err(super::invalid_request("unsafe bootstrap progress"));
    }
    let mut bytes = Vec::new();
    file.take((MAX_BOOTSTRAP_PROGRESS + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BOOTSTRAP_PROGRESS {
        return Err(super::invalid_request("oversized bootstrap progress"));
    }
    let progress: BootstrapProgress =
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
    if serde_json::from_slice::<serde_json::Value>(&bytes).map_err(std::io::Error::other)?
        != serde_json::to_value(&progress).map_err(std::io::Error::other)?
        || progress.version != 1
        || &progress.identity != identity
        || (!progress.possible_creation
            && (progress.request.is_some()
                || progress.creation.is_some()
                || progress.not_submitted))
        || (progress.not_submitted && progress.creation.is_some())
    {
        return Err(super::invalid_request(
            "bootstrap progress identity or state differs",
        ));
    }
    if let Some(request) = &progress.request
        && (request.workspace != identity.payload.workspace
            || request.cwd != identity.payload.cwd
            || request.label != identity.payload.label
            || request.focus != identity.payload.focus
            || request.env != identity.payload.env
            || request.expected_witness.endpoint
                != identity.payload.handoff.namespace.host_endpoint)
    {
        return Err(super::invalid_request(
            "bootstrap saved native request differs",
        ));
    }
    if let Some(created) = &progress.creation {
        created.validate().map_err(super::invalid_request)?;
        if created.workspace != identity.payload.workspace
            || created.witness.endpoint != identity.payload.handoff.namespace.host_endpoint
            || progress.request.as_ref().is_some_and(|r| {
                r.correlation != created.correlation || r.expected_witness != created.witness
            })
        {
            return Err(super::invalid_request("bootstrap saved creation differs"));
        }
    }
    Ok(Some(progress))
}
fn bootstrap_result(
    identity: &crate::protocol::handoff::BootstrapIdentity,
    result: crate::protocol::results::CommandResult,
) -> Result<crate::protocol::handoff::BootstrapResult, RunError> {
    use crate::protocol::{handoff::*, results::CommandResult};
    let CommandResult::Bootstrap(result) = result else {
        return Err(super::invalid_request("unexpected bootstrap result"));
    };
    if result.compound != identity.compound {
        return Err(super::invalid_request(
            "canonical bootstrap compound differs",
        ));
    }
    if let Some(created) = &result.creation {
        created.validate().map_err(super::invalid_request)?;
        if created.workspace != identity.payload.workspace
            || created.witness.endpoint != identity.payload.handoff.namespace.host_endpoint
        {
            return Err(super::invalid_request(
                "canonical creation namespace differs",
            ));
        }
    }
    if let Some(attachment) = &result.attachment {
        attachment
            .validate(identity)
            .map_err(super::invalid_request)?;
        if attachment.attempt != result.attempt
            || result.creation.as_ref() != Some(&attachment.created)
        {
            return Err(super::invalid_request(
                "canonical attachment evidence differs",
            ));
        }
    }
    if matches!(
        result.state,
        BootstrapState::Created | BootstrapState::Attached | BootstrapState::Completed
    ) && result.creation.is_none()
    {
        return Err(super::invalid_request(
            "canonical bootstrap lacks creation evidence",
        ));
    }
    Ok(*result)
}
fn record_creation(
    identity: &crate::protocol::handoff::BootstrapIdentity,
    attempt: crate::protocol::handoff::BootstrapAttempt,
    evidence: &crate::ports::CreatedTab,
    call: &impl Fn(
        crate::protocol::commands::Command,
    ) -> Result<crate::protocol::results::CommandResult, RunError>,
    status: &impl Fn() -> Result<crate::protocol::handoff::BootstrapResult, RunError>,
) -> Result<crate::protocol::handoff::BootstrapResult, RunError> {
    use crate::protocol::{commands::Command, handoff::*};
    match call(Command::RecordBootstrapCreated(Box::new(
        RecordBootstrapCreated {
            identity: identity.clone(),
            expected_attempt: attempt,
            operation: attempt
                .operation(&identity.compound, "record")
                .map_err(super::invalid_request)?,
            evidence: evidence.clone(),
        },
    ))) {
        Ok(result) => bootstrap_result(identity, result),
        Err(error) => match status() {
            Ok(saved) if saved.attempt == attempt && saved.creation.as_ref() == Some(evidence) => {
                Ok(saved)
            }
            _ => Err(error),
        },
    }
}
fn close_not_submitted(
    identity: &crate::protocol::handoff::BootstrapIdentity,
    attempt: crate::protocol::handoff::BootstrapAttempt,
    call: &impl Fn(
        crate::protocol::commands::Command,
    ) -> Result<crate::protocol::results::CommandResult, RunError>,
) -> Result<crate::protocol::handoff::BootstrapResult, RunError> {
    use crate::protocol::{commands::Command, handoff::*};
    let result = bootstrap_result(
        identity,
        call(Command::RecordBootstrapNotSubmitted(Box::new(
            RecordBootstrapNotSubmitted {
                identity: identity.clone(),
                expected_attempt: attempt,
                operation: attempt
                    .operation(&identity.compound, "not_submitted")
                    .map_err(super::invalid_request)?,
            },
        )))?,
    )?;
    if result.attempt.get() <= attempt.get() {
        return Err(super::invalid_request(
            "zero submission did not allocate a distinct attempt",
        ));
    }
    Ok(result)
}
fn creation_unknown(
    reference: &super::journal::IntentRef,
    identity: &crate::protocol::handoff::BootstrapIdentity,
    attempt: crate::protocol::handoff::BootstrapAttempt,
) -> RunError {
    crate::protocol::results::ApiError::unknown_outcome(format!("bootstrap {} compound {} attempt {}: outcome_unknown; inspect original exact namespace before guarded recovery; no automatic creation", reference.recovery_ref(), identity.compound.as_str(), attempt.get())).into()
}

#[cfg(test)]
#[path = "../../tests/cli/topology_handoff.rs"]
mod tests;
