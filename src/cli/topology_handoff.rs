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
            harness: harness.occupant(),
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
    preflight_launch_argv(&identity)?;
    Ok(identity)
}

/// Fresh public preparation only: never replace or reinterpret a retained original.
pub(super) fn preflight_launch_request(
    identity: &crate::protocol::handoff::BootstrapIdentity,
) -> Result<super::launch::LaunchRequest, RunError> {
    use crate::protocol::{handoff::HandoffChannel, ids::ThreadId};
    let namespace = &identity.payload.handoff.namespace;
    let context = crate::protocol::output::ContinuationContext {
        state_dir: Some(namespace.state_dir.to_string_lossy().into_owned()),
        host: Some(namespace.host_endpoint.to_string_lossy().into_owned()),
    };
    // A new thread's identifier is not allocated yet. Reserve the longest legal
    // identifier; no future random identifier may make the saved argv unlaunchable.
    let longest = ThreadId::new("t".repeat(128));
    let thread = match &identity.payload.handoff.channel {
        HandoffChannel::Existing { thread } => thread,
        HandoffChannel::New { .. } => &longest,
    };
    let mut argv = identity.payload.launch.argv.clone();
    argv.push(super::handoff::bootstrap_v1::prompt(
        thread.as_str(),
        &context,
        &identity.claim.instance,
    ));
    Ok(super::launch::LaunchRequest {
        target: identity.claim.target.clone(),
        harness: identity.payload.launch.harness.into(),
        harness_binary: identity.payload.launch.binary.clone(),
        name: identity.payload.launch.name.clone(),
        pane_label: Some(identity.payload.label.clone()),
        argv,
    })
}
fn preflight_launch_argv(
    identity: &crate::protocol::handoff::BootstrapIdentity,
) -> Result<(), RunError> {
    let request = preflight_launch_request(identity)?;
    // Supported production hooks own no native args. This is the same composition
    // and geometry used by prepare_managed, including Codex grammar validation.
    let argv = crate::harness::launch::compose_native_argv(
        identity.payload.launch.harness,
        request.argv.clone(),
        vec![],
    )?;
    // Completion validates the retained report against frozen V1 composition.
    if argv
        != crate::harness::launch::compose_bootstrap_v1_argv(
            identity.payload.launch.harness,
            request.argv,
        )?
    {
        return Err(super::invalid_request(
            "current launch composition differs from bootstrap plan V1; a new bootstrap plan version is required",
        ));
    }
    crate::ports::NativeLaunchRequest::validate_argv(&argv).map_err(super::invalid_request)?;
    // The retained successful report has a tighter per-argument ceiling than
    // the native transport, including its generated prompt. Keep both contracts.
    if argv.iter().any(|arg| arg.len() > 4096) {
        return Err(super::invalid_request(
            "bootstrap native argument exceeds retained report limit",
        ));
    }
    Ok(())
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
        // Canonical operator evidence is authority for downstream attachment,
        // not a receipt of this client's original native submission. Retain
        // the genuine saved request/witness and any matching actual receipt.
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
    let harness = Harness::from(payload.launch.harness);
    let plan = super::handoff::HandoffPlan {
        startup_input: None,
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
/// Caller holds the original operation lock; retained bytes never become authority.
pub(crate) fn saved_native_request(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
    identity: &crate::protocol::handoff::BootstrapIdentity,
) -> Result<Option<crate::ports::CreateTabRequest>, RunError> {
    Ok(
        load_bootstrap_progress(journal, reference, identity)?
            .and_then(|progress| progress.request),
    )
}
/// Before publication, check the known eventual request-bearing envelope with
/// the actual witnessed endpoint metadata and the actual serializer/cap.
pub(crate) fn preflight_progress_capacity(
    identity: &crate::protocol::handoff::BootstrapIdentity,
    witness: &crate::host::continuity::LocalEndpointWitness,
) -> Result<(), RunError> {
    let preview = BootstrapProgress {
        version: 1,
        identity: identity.clone(),
        attempt: crate::protocol::handoff::BootstrapAttempt::first(),
        possible_creation: true,
        request: Some(crate::ports::CreateTabRequest {
            correlation: crate::protocol::ids::HostCallId::new(uuid::Uuid::new_v4().to_string()),
            workspace: identity.payload.workspace.clone(),
            cwd: identity.payload.cwd.clone(),
            label: identity.payload.label.clone(),
            focus: identity.payload.focus,
            env: identity.payload.env.clone(),
            expected_witness: witness.clone(),
        }),
        creation: None,
        not_submitted: false,
    };
    if serde_json::to_vec(&preview)
        .map_err(std::io::Error::other)?
        .len()
        > MAX_BOOTSTRAP_PROGRESS
    {
        return Err(super::invalid_request(
            "bootstrap request-bearing progress exceeds retained byte cap",
        ));
    }
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
        || (progress.not_submitted && progress.request.is_none())
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
pub(crate) fn creation_unknown(
    reference: &super::journal::IntentRef,
    identity: &crate::protocol::handoff::BootstrapIdentity,
    attempt: crate::protocol::handoff::BootstrapAttempt,
) -> RunError {
    crate::protocol::results::ApiError::unknown_outcome(format!("bootstrap {} compound {} attempt {}: outcome_unknown; inspect original exact namespace before guarded recovery; no automatic creation", reference.recovery_ref(), identity.compound.as_str(), attempt.get())).into()
}

// Linked child progress is separate from the bootstrap submission record. Local
// bytes only conservatively fence launch; canonical completion owns the report.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChildProgress {
    version: u32,
    pub(crate) identity: crate::protocol::handoff::BootstrapIdentity,
    pub(crate) attachment: crate::protocol::handoff::BootstrapAttachment,
    pub(crate) progress: super::handoff::Progress,
}
fn child_progress_path(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
) -> std::path::PathBuf {
    journal.root().join(format!(
        "bootstrap-child-{}.progress",
        reference.operation.as_str()
    ))
}
// Canonical identity + attachment + genuine raw launch report and bounded staged
// metadata fit the child ceiling. Terminal includes the full canonical envelope
// plus original JSON escaped once as a String (at most twice its compact bytes).
pub(crate) const MAX_LINKED_LOCAL_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_BOOTSTRAP_TERMINAL_BYTES: usize =
    crate::store::topology_handoff::MAX_COMPLETED_BYTES
        + 2 * super::journal::MAX_BOOTSTRAP_ORIGIN_BYTES
        + 4096;
fn read_linked_bytes(path: &std::path::Path, limit: usize) -> Result<Option<Vec<u8>>, RunError> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit as u64 {
        return Err(super::invalid_request(
            "unsafe or oversized bootstrap retained record",
        ));
    }
    let mut bytes = vec![];
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(super::invalid_request(
            "oversized bootstrap retained record",
        ));
    }
    Ok(Some(bytes))
}
fn decode_linked<T: serde::de::DeserializeOwned + serde::Serialize>(
    bytes: &[u8],
    limit: usize,
) -> Result<T, RunError> {
    if bytes.len() > limit {
        return Err(super::invalid_request(
            "oversized bootstrap retained record",
        ));
    }
    let value: T = serde_json::from_slice(bytes).map_err(std::io::Error::other)?;
    if serde_json::from_slice::<serde_json::Value>(bytes).map_err(std::io::Error::other)?
        != serde_json::to_value(&value).map_err(std::io::Error::other)?
    {
        return Err(super::invalid_request(
            "unexpected bootstrap retained fields",
        ));
    }
    Ok(value)
}

fn same_record<T: serde::Serialize>(a: &T, b: &T) -> Result<bool, RunError> {
    Ok(serde_json::to_value(a).map_err(std::io::Error::other)?
        == serde_json::to_value(b).map_err(std::io::Error::other)?)
}
fn load_child_progress(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
    identity: &crate::protocol::handoff::BootstrapIdentity,
    attachment: &crate::protocol::handoff::BootstrapAttachment,
) -> Result<super::handoff::Progress, RunError> {
    let Some(bytes) = read_linked_bytes(
        &child_progress_path(journal, reference),
        MAX_LINKED_LOCAL_BYTES,
    )?
    else {
        return Ok(Default::default());
    };
    let saved = decode_child_progress(&bytes)?;
    if !same_record(&saved.identity, identity)? || !same_record(&saved.attachment, attachment)? {
        return Err(super::invalid_request(
            "bootstrap child progress identity or state differs",
        ));
    }
    Ok(saved.progress)
}
/// Pure owning-module decoder; local evidence never proves canonical attachment.
pub(crate) fn decode_child_progress(bytes: &[u8]) -> Result<ChildProgress, RunError> {
    let saved: ChildProgress = decode_linked(bytes, MAX_LINKED_LOCAL_BYTES)?;
    attached_handoff_plan(&saved.identity, &saved.attachment)?;
    if saved.version != 1
        || (saved.progress.possible_start
            && (saved.progress.thread.is_none()
                || saved.progress.invitation.is_none()
                || saved.progress.message.is_none()))
        || (saved.progress.launch.is_some() && !saved.progress.possible_start)
        || saved
            .progress
            .thread
            .as_ref()
            .zip(saved.attachment.handoff.thread.as_ref())
            .is_some_and(|(a, b)| a != b)
        || saved.progress.invitation.as_ref().is_some_and(|v| {
            !matches!(
                v,
                crate::protocol::results::CommandResult::Invitation(_)
                    | crate::protocol::results::CommandResult::AlreadyJoined(_)
            )
        })
        || saved
            .progress
            .message
            .as_ref()
            .is_some_and(|v| !matches!(v, crate::protocol::results::CommandResult::MessageSent(_)))
    {
        return Err(super::invalid_request(
            "bootstrap child progress identity or state differs",
        ));
    }
    Ok(saved)
}
pub(crate) fn bootstrap_identity(
    pending: &super::journal::PendingIntent,
) -> Result<crate::protocol::handoff::BootstrapIdentity, RunError> {
    let super::journal::SemanticMutation::Frozen { claim, mutation } = &pending.semantic else {
        return Err(super::invalid_request(
            "bootstrap needs original frozen caller",
        ));
    };
    let super::journal::SemanticMutation::HandoffBootstrap(plan) = mutation.as_ref() else {
        return Err(super::invalid_request("not a bootstrap origin"));
    };
    let identity = crate::protocol::handoff::BootstrapIdentity {
        compound: plan.payload.handoff.keys.compound.clone(),
        scope: pending.header.scope.clone(),
        claim: claim.clone(),
        digest: pending.header.semantic_digest.clone(),
        payload: plan.payload.clone(),
    };
    identity.validate().map_err(super::invalid_request)?;
    Ok(identity)
}
fn validate_completed(
    identity: &crate::protocol::handoff::BootstrapIdentity,
    done: &crate::protocol::handoff::CompletedBootstrapResult,
) -> Result<(), RunError> {
    use crate::protocol::handoff::*;
    let wrapper = CompleteLinkedBootstrap {
        identity: identity.clone(),
        attachment: done.attachment.clone(),
        operation: identity.payload.linked_complete_key.clone(),
        legacy_completion: HandoffMutation {
            identity: done.attachment.handoff.clone(),
            operation: identity.payload.handoff.keys.complete.clone(),
        },
        retained: done.retained.clone(),
    };
    wrapper.validate().map_err(super::invalid_request)?;
    let plan = attached_handoff_plan(identity, &done.attachment)?;
    // Frozen V1 composition, never today's renderer or registry.
    let launched = super::handoff::bootstrap_v1::retained_argv_matches(
        identity.payload.launch.harness,
        &plan.request.launch.argv,
        done.retained.thread.as_str(),
        &plan.context,
        &identity.claim.instance,
        done.retained.report.get("argv"),
    )?;
    if !same_record(&done.identity, identity)?
        || done.legacy_result.state != HandoffState::Completed
        || done.legacy_result.compound != done.attachment.handoff.compound
        || done.legacy_result.thread.as_ref() != Some(&done.retained.thread)
        || !launched
    {
        return Err(super::invalid_request("bootstrap completed report differs"));
    }
    Ok(())
}
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BootstrapTerminal {
    version: u32,
    pub(crate) original: String,
    pub(crate) completed: crate::protocol::handoff::CompletedBootstrapResult,
}
fn terminal_path(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
) -> std::path::PathBuf {
    journal.root().join(format!(
        "bootstrap-{:020}-{}.terminal",
        reference.ordinal,
        reference.operation.as_str()
    ))
}
fn read_terminal(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
) -> Result<Option<BootstrapTerminal>, RunError> {
    let Some(bytes) = read_linked_bytes(
        &terminal_path(journal, reference),
        MAX_BOOTSTRAP_TERMINAL_BYTES,
    )?
    else {
        return Ok(None);
    };
    let saved = decode_bootstrap_terminal(reference, &bytes)?;
    match journal.snapshot_bootstrap_origin(reference) {
        Ok(original) if original != saved.original.as_bytes() => {
            return Err(super::invalid_request(
                "bootstrap terminal origin contradiction",
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(Some(saved))
}
/// Pure retained terminal decoder shared by pinned archival reads and producer paths.
pub(crate) fn decode_bootstrap_terminal(
    reference: &super::journal::IntentRef,
    bytes: &[u8],
) -> Result<BootstrapTerminal, RunError> {
    let saved: BootstrapTerminal = decode_linked(bytes, MAX_BOOTSTRAP_TERMINAL_BYTES)?;
    if saved.version != 1 {
        return Err(super::invalid_request(
            "unsupported bootstrap terminal record",
        ));
    }
    let pending =
        super::journal::Journal::decode_bootstrap_origin(reference, saved.original.as_bytes())?;
    validate_completed(&bootstrap_identity(&pending)?, &saved.completed)?;
    Ok(saved)
}
/// Bounded read-only original evidence for the shared classifier. Retained
/// terminal bytes only select the exact origin; they authorize no live effect.
pub(crate) fn load_original(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
) -> Result<super::journal::PendingIntent, RunError> {
    if let Some(saved) = read_terminal(journal, reference)? {
        return Ok(super::journal::Journal::decode_bootstrap_origin(
            reference,
            saved.original.as_bytes(),
        )?);
    }
    let bytes = journal.snapshot_bootstrap_origin(reference)?;
    let pending = super::journal::Journal::decode_bootstrap_origin(reference, &bytes)?;
    bootstrap_identity(&pending)?;
    Ok(pending)
}
pub(crate) fn resolve_recovery_ref(
    journal: &super::journal::Journal,
    value: &str,
) -> Result<super::journal::IntentRef, RunError> {
    let ordinal: u64 = value
        .strip_prefix("local:")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| super::invalid_request("invalid bootstrap reference"))?;
    if value != format!("local:{ordinal}") {
        return Err(super::invalid_request("noncanonical bootstrap reference"));
    }
    let mut found = None;
    for entry in std::fs::read_dir(journal.root())? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if let Some(operation) = name
            .strip_prefix(&format!("bootstrap-{ordinal:020}-"))
            .and_then(|v| v.strip_suffix(".terminal"))
            .or_else(|| {
                name.strip_prefix(&format!("{ordinal:020}-"))
                    .and_then(|v| v.strip_suffix(".intent"))
            })
        {
            let reference = super::journal::IntentRef {
                ordinal,
                operation: crate::protocol::ids::OperationId::parse(operation.to_owned())
                    .map_err(super::invalid_request)?,
            };
            load_original(journal, &reference)?;
            if found
                .as_ref()
                .is_some_and(|previous| previous != &reference)
            {
                return Err(super::invalid_request("ambiguous bootstrap reference"));
            }
            found = Some(reference);
        }
    }
    found.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "bootstrap reference not found",
        )
        .into()
    })
}
fn terminal_bytes(terminal: &BootstrapTerminal) -> Result<Vec<u8>, RunError> {
    let bytes = serde_json::to_vec(terminal).map_err(std::io::Error::other)?;
    if bytes.len() > MAX_BOOTSTRAP_TERMINAL_BYTES {
        return Err(super::invalid_request(
            "oversized bootstrap terminal record",
        ));
    }
    Ok(bytes)
}
fn save_terminal(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
    terminal: &BootstrapTerminal,
) -> Result<(), RunError> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let bytes = terminal_bytes(terminal)?;
    let temp = journal
        .root()
        .join(format!(".bootstrap-terminal-{}", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::hard_link(&temp, terminal_path(journal, reference))?;
        std::fs::File::open(journal.root())?.sync_all()
    })();
    let cleanup = std::fs::remove_file(temp);
    result?;
    cleanup?;
    std::fs::File::open(journal.root())?.sync_all()?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn resume_to_writer<
    C: crate::ports::LocalClient + ?Sized,
    N: crate::ports::CreateTabPort + ?Sized,
    W: std::io::Write,
>(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
    actor: super::actor_route::InvocationActor,
    namespace: &crate::protocol::handoff::HandoffNamespace,
    client: &C,
    native: &N,
    launcher: &mut dyn super::handoff::HandoffLauncher,
    clock: &dyn crate::protocol::time::Clock,
    submission: BootstrapSubmissionInputs<'_>,
    output: &crate::protocol::output::OutputSpec,
    writer: &mut W,
) -> Result<crate::protocol::handoff::BootstrapResult, RunError> {
    use crate::protocol::{commands::Command, handoff::*, results::CommandResult};
    let pending = load_original(journal, reference)?;
    let identity = bootstrap_identity(&pending)?;
    crate::store::topology_handoff::encode_identity(namespace, &identity)?;
    let status = || -> Result<BootstrapResult, RunError> {
        bootstrap_result(
            &identity,
            client.call(
                Command::BootstrapStatus(Box::new(BootstrapStatus {
                    identity: identity.clone(),
                })),
                &super::cooperative_budget(clock),
            )?,
        )
    };
    let retained = read_terminal(journal, reference)?;
    let current = match status() {
        Ok(current) => Some(current),
        Err(RunError::Api(e))
            if e.code == crate::protocol::results::ErrorCode::NotFound && retained.is_none() =>
        {
            None
        }
        Err(e) => return Err(e),
    };
    if retained.is_some()
        && current
            .as_ref()
            .is_none_or(|v| v.state != BootstrapState::Completed)
    {
        return Err(super::invalid_request(
            "bootstrap terminal lacks canonical completion",
        ));
    }
    if current.as_ref().is_none_or(|v| {
        !matches!(
            v.state,
            BootstrapState::Attached | BootstrapState::Completed
        )
    }) && let Err(error) = super::retry::run_bootstrap_retry(
        journal, reference, actor, namespace, client, native, clock, submission,
    ) {
        if reportable_failure(&error)
            && let Ok(current) = status()
        {
            write_pending(
                reference, &identity, &current, None, "creation", output, writer,
            )?;
        }
        return Err(error);
    }
    let _lock = super::handoff::lock(journal, reference)?;
    let retained = read_terminal(journal, reference)?;
    let mut current = status()?;
    if retained.is_some() && current.state != BootstrapState::Completed {
        return Err(super::invalid_request(
            "bootstrap terminal lacks canonical completion",
        ));
    }
    if current.state != BootstrapState::Completed {
        let attachment = current
            .attachment
            .as_ref()
            .filter(|_| current.state == BootstrapState::Attached)
            .ok_or_else(|| super::invalid_request("bootstrap lacks canonical attachment"))?;
        let plan = attached_handoff_plan(&identity, attachment)?;
        let child = super::handoff::keyed_fence(
            client,
            clock,
            &attachment.handoff,
            identity.payload.handoff.keys.begin.clone(),
            false,
        )?;
        if child.state != HandoffState::Live {
            return Err(super::invalid_request(
                "bootstrap child completed without atomic parent completion",
            ));
        }
        let mut progress = load_child_progress(journal, reference, &identity, attachment)?;
        if progress
            .thread
            .as_ref()
            .zip(child.thread.as_ref())
            .is_some_and(|(a, b)| a != b)
        {
            return Err(super::invalid_request(
                "bootstrap child progress contradicts canonical thread",
            ));
        }
        let (phase, attempt) = super::handoff::execute_steps(
            &plan,
            &identity.claim,
            &mut progress,
            &mut |p| {
                let saved = ChildProgress {
                    version: 1,
                    identity: identity.clone(),
                    attachment: attachment.clone(),
                    progress: p.clone(),
                };
                if serde_json::to_vec(&saved)
                    .map_err(std::io::Error::other)?
                    .len()
                    > MAX_LINKED_LOCAL_BYTES
                {
                    return Err(super::invalid_request("oversized bootstrap child progress"));
                }
                super::handoff::save_progress_at(
                    journal,
                    &child_progress_path(journal, reference),
                    &saved,
                )?;
                Ok(())
            },
            client,
            launcher,
            clock,
            &mut || {
                // Live Begin replay is a deciding A2 check, not cached admission.
                // This is bounded cooperative freshness, not a daemon/host transaction.
                match client.call(
                    Command::BeginHandoff(HandoffMutation {
                        identity: attachment.handoff.clone(),
                        operation: identity.payload.handoff.keys.begin.clone(),
                    }),
                    &super::cooperative_budget(clock),
                )? {
                    CommandResult::Handoff(result)
                        if result.compound == attachment.handoff.compound
                            && result.state == HandoffState::Live
                            && result.thread.is_some() =>
                    {
                        Ok(())
                    }
                    _ => Err(crate::protocol::results::ApiError::invalid_request(
                        "bootstrap child live launch guard differs",
                    )),
                }
            },
            None,
        );
        if progress.possible_start
            && progress
                .launch
                .as_ref()
                .is_none_or(|v| v["outcome"] != "started")
        {
            write_pending(
                reference,
                &identity,
                &current,
                Some(&progress),
                phase,
                output,
                writer,
            )?;
            return Err(crate::protocol::results::ApiError::unknown_outcome(format!("bootstrap {} downstream possible start; inspect exact pane {} and seat {}; no automatic launch",reference.recovery_ref(),attachment.created.root_pane.as_str(),attachment.resolved_seat.as_str())).into());
        }
        if let Err(error) = attempt {
            if reportable_failure(&error) {
                write_pending(
                    reference,
                    &identity,
                    &current,
                    Some(&progress),
                    phase,
                    output,
                    writer,
                )?;
            }
            return Err(error);
        }
        if let Err(error) = super::handoff::complete_with(
            &attachment.handoff,
            identity.payload.handoff.keys.complete.clone(),
            &progress,
            &mut |legacy_completion, thread, report| {
                use sha2::{Digest, Sha256};
                let retained = LinkedBootstrapReport {
                    thread: thread.clone(),
                    recipient: attachment.resolved_seat.clone(),
                    pane: attachment.created.root_pane.clone(),
                    kind: "launch".into(),
                    launch: identity.payload.launch.clone(),
                    report: report.clone(),
                    report_digest: format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(report).map_err(std::io::Error::other)?)
                    ),
                    terminal: attachment.created.terminal.clone(),
                    host_incarnation: attachment.created.host_incarnation.clone(),
                };
                let wrapper = CompleteLinkedBootstrap {
                    identity: identity.clone(),
                    attachment: attachment.clone(),
                    operation: identity.payload.linked_complete_key.clone(),
                    legacy_completion,
                    retained,
                };
                wrapper.validate().map_err(super::invalid_request)?;
                // This preview checks only the eventual envelope's size/shape;
                // presentation uses the actual canonical response/status below.
                let preview = CompletedBootstrapResult {
                    identity: identity.clone(),
                    attachment: attachment.clone(),
                    retained: wrapper.retained.clone(),
                    legacy_result: HandoffResult {
                        compound: attachment.handoff.compound.clone(),
                        thread: Some(thread.clone()),
                        state: HandoffState::Completed,
                    },
                };
                validate_completed(&identity, &preview)?;
                if serde_json::to_vec(&preview)
                    .map_err(std::io::Error::other)?
                    .len()
                    > crate::store::topology_handoff::MAX_COMPLETED_BYTES
                {
                    return Err(super::invalid_request(
                        "oversized bootstrap completion report",
                    ));
                }
                terminal_bytes(&BootstrapTerminal {
                    version: 1,
                    original: String::from_utf8(journal.snapshot_bootstrap_origin(reference)?)
                        .map_err(std::io::Error::other)?,
                    completed: preview,
                })?;
                let result = client.call(
                    Command::CompleteLinkedBootstrap(Box::new(wrapper.clone())),
                    &super::cooperative_budget(clock),
                );
                let done = match result {
                    Ok(CommandResult::LinkedBootstrapCompleted(done)) => *done,
                    Ok(_) => {
                        return Err(super::invalid_request(
                            "unexpected linked completion result",
                        ));
                    }
                    Err(error) => {
                        let saved = status()?;
                        match saved.completed {
                            Some(done)
                                if saved.state == BootstrapState::Completed
                                    && done.retained == wrapper.retained =>
                            {
                                *done
                            }
                            _ => return Err(error.into()),
                        }
                    }
                };
                validate_completed(&identity, &done)?;
                if done.attachment != wrapper.attachment || done.retained != wrapper.retained {
                    return Err(super::invalid_request("linked completion report differs"));
                }
                Ok(())
            },
        ) {
            if reportable_failure(&error) {
                write_pending(
                    reference,
                    &identity,
                    &current,
                    Some(&progress),
                    "complete",
                    output,
                    writer,
                )?;
            }
            return Err(error);
        }
        current = status()?;
    }
    present_completed(
        journal, reference, &identity, current, retained, output, writer,
    )
}

fn reportable_failure(error: &RunError) -> bool {
    use crate::protocol::results::ErrorCode;
    !matches!(error, RunError::Api(e) if matches!(e.code,
        ErrorCode::Unauthorized | ErrorCode::CallerUnverified | ErrorCode::InstanceMismatch
        | ErrorCode::OperationPayloadMismatch))
}

/// Presentation of admitted history only. These commands confer no admission,
/// ownership, noncreation or quiescence; every deciding path rechecks its guards.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_pending<W: std::io::Write>(
    reference: &super::journal::IntentRef,
    identity: &crate::protocol::handoff::BootstrapIdentity,
    current: &crate::protocol::handoff::BootstrapResult,
    progress: Option<&super::handoff::Progress>,
    phase: &str,
    output: &crate::protocol::output::OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    use crate::protocol::handoff::BootstrapState;
    if current.compound != identity.compound
        || matches!(
            current.state,
            BootstrapState::Completed | BootstrapState::Cancelled
        )
    {
        return Err(super::invalid_request("pending bootstrap status differs"));
    }
    let namespace = &identity.payload.handoff.namespace;
    let context = crate::protocol::output::ContinuationContext {
        state_dir: Some(namespace.state_dir.to_string_lossy().into_owned()),
        host: Some(namespace.host_endpoint.to_string_lossy().into_owned()),
    };
    let prefix = super::hook::cli_prefix(&context);
    let mut retry = vec![
        "env".into(),
        format!("HERDR_PANE_ID={}", identity.claim.target.as_str()),
    ];
    retry.extend(prefix.clone());
    retry.extend(["retry".into(), reference.recovery_ref()]);
    let mut inspect = prefix.clone();
    inspect.push("pending".into());
    let unknown_creation =
        current.state == BootstrapState::PossibleCreation && current.creation.is_none();
    let possible_start = progress.is_some_and(|p| {
        p.possible_start && p.launch.as_ref().is_none_or(|v| v["outcome"] != "started")
    });
    let started =
        progress.is_some_and(|p| p.launch.as_ref().is_some_and(|v| v["outcome"] == "started"));
    let mut report = serde_json::json!({
        "phase":phase,"failed":true,
        "outcome":if unknown_creation {"creation_unknown"} else if possible_start {"possible_start"} else if started {"completion_pending"} else {"pending"},
        "namespace":namespace,"recovery_ref":reference.recovery_ref(),"bootstrap_compound":identity.compound,
        "attempt":current.attempt,"state":current.state,"attempt_state":current.attempt_state,"status_is_last_observed":true,
        "workspace":identity.payload.workspace,"creation":current.creation,
        "tab":current.creation.as_ref().map(|v| &v.tab),"pane":current.creation.as_ref().map(|v| &v.root_pane),
        "seat":current.attachment.as_ref().map(|v| &v.resolved_seat),
        "child_compound":current.attachment.as_ref().map(|v| &v.handoff.compound),
        "possible_start":possible_start,"thread":null,"invitation":null,"message":null,
        "retry_argv":retry,"inspect_argv":inspect,
        "manual_launch_after_confirming_no_start_argv":null,"conditional_human_recovery":null,
        "guidance":"Fields are last confirmed observations; null means unknown, not proof of nonexecution. Inspect this exact namespace and attempt. Retry never proves noncreation or no start. Do not launch manually unless inspection confirms no start. No automatic topology cleanup, launch, invitation acceptance or ACK."
    });
    if let (Some(attachment), Some(progress)) = (&current.attachment, progress) {
        let plan = attached_handoff_plan(identity, attachment)?;
        let legacy = super::handoff::report(
            reference,
            &plan,
            progress,
            phase,
            true,
            possible_start,
            &identity.claim,
        );
        for key in ["thread", "invitation", "message"] {
            report[key] = legacy[key].clone();
        }
        report["seat_inspect_argv"] = legacy["inspect_argv"].clone();
        if !started && progress.thread.is_some() {
            report["manual_launch_after_confirming_no_start_argv"] =
                legacy["manual_launch_after_confirming_no_start_argv"].clone();
        }
    }
    if unknown_creation {
        let mut human = prefix;
        human.insert(1, "human".into());
        human.extend([
            "handoff".into(),
            "recover".into(),
            reference.recovery_ref(),
            "--attempt".into(),
            current.attempt.get().to_string(),
        ]);
        let mut created = human.clone();
        created.extend([
            "--created-pane".into(),
            "REPLACE_WITH_EXACT_INSPECTED_PANE".into(),
        ]);
        let mut absent = human.clone();
        absent.push("--not-created".into());
        human.extend([
            "--cancel".into(),
            "--reason".into(),
            "REPLACE_WITH_INSPECTED_QUIESCENCE_REASON".into(),
        ]);
        report["conditional_human_recovery"] = serde_json::json!({
            "conditions":"The operator must first inspect exact original namespace/attempt and exclude an in-flight invocation. Use created-pane only for an inspected exact pane; not-created only after confirming noncreation and quiescence; cancel only after confirming quiescence and no protected downstream child. These templates are alternatives, not automatic actions.",
            "created_pane_argv":created,"not_created_argv":absent,"cancel_argv":human
        });
    }
    let bytes = if output.format == crate::protocol::output::OutputFormat::Json {
        format!("{}\n", serde_json::json!({"bootstrap":report})).into_bytes()
    } else {
        // Render commands with real shell quoting, retaining JSON argv arrays above.
        let map = report.as_object_mut().unwrap();
        for (key, value) in map.iter_mut() {
            if key.ends_with("_argv")
                && let Ok(argv) = serde_json::from_value::<Vec<String>>(value.clone())
            {
                *value = crate::protocol::output::format_command_argv(&argv).into();
            }
        }
        if let Some(recovery) = report["conditional_human_recovery"].as_object_mut() {
            for (key, value) in recovery.iter_mut() {
                if key.ends_with("_argv")
                    && let Ok(argv) = serde_json::from_value::<Vec<String>>(value.clone())
                {
                    *value = crate::protocol::output::format_command_argv(&argv).into();
                }
            }
        }
        super::setup::render_text(&report).into_bytes()
    };
    if bytes.len() > 1024 * 1024 {
        return Err(super::invalid_request("oversized bootstrap pending report"));
    }
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

/// Exact terminal presentation precedes every fresh host or caller read.
pub(crate) fn try_completed<C: crate::ports::LocalClient + ?Sized, W: std::io::Write>(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
    namespace: &crate::protocol::handoff::HandoffNamespace,
    client: &C,
    clock: &dyn crate::protocol::time::Clock,
    output: &crate::protocol::output::OutputSpec,
    writer: &mut W,
) -> Result<bool, RunError> {
    use crate::protocol::{commands::Command, handoff::*, results::ErrorCode};
    let pending = load_original(journal, reference)?;
    if super::journal::classify_original_actor(&pending.header.scope, &pending.semantic)?
        != super::journal::OriginalActor::Agent
    {
        return Err(super::invalid_request("bootstrap needs original agent"));
    }
    let identity = bootstrap_identity(&pending)?;
    crate::store::topology_handoff::encode_identity(namespace, &identity)?;
    let _lock = super::handoff::lock(journal, reference)?;
    let retained = read_terminal(journal, reference)?;
    let current = match client.call(
        Command::BootstrapStatus(Box::new(BootstrapStatus {
            identity: identity.clone(),
        })),
        &super::cooperative_budget(clock),
    ) {
        Ok(result) => bootstrap_result(&identity, result)?,
        Err(error) if error.code == ErrorCode::NotFound && retained.is_none() => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if current.state != BootstrapState::Completed {
        if retained.is_some() {
            return Err(super::invalid_request(
                "bootstrap terminal lacks canonical completion",
            ));
        }
        return Ok(false);
    }
    present_completed(
        journal, reference, &identity, current, retained, output, writer,
    )?;
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn present_completed<W: std::io::Write>(
    journal: &super::journal::Journal,
    reference: &super::journal::IntentRef,
    identity: &crate::protocol::handoff::BootstrapIdentity,
    current: crate::protocol::handoff::BootstrapResult,
    retained: Option<BootstrapTerminal>,
    output: &crate::protocol::output::OutputSpec,
    writer: &mut W,
) -> Result<crate::protocol::handoff::BootstrapResult, RunError> {
    use crate::protocol::handoff::*;
    let done = current
        .completed
        .as_ref()
        .filter(|_| current.state == BootstrapState::Completed)
        .ok_or_else(|| {
            super::invalid_request("completed bootstrap requires retained successful report")
        })?;
    validate_completed(identity, done)?;
    if let Some(terminal) = retained {
        if !same_record(&terminal.completed, done.as_ref())? {
            return Err(super::invalid_request(
                "bootstrap terminal disagrees with canonical completion",
            ));
        }
    } else {
        let terminal = BootstrapTerminal {
            version: 1,
            original: String::from_utf8(journal.snapshot_bootstrap_origin(reference)?)
                .map_err(std::io::Error::other)?,
            completed: (**done).clone(),
        };
        save_terminal(journal, reference, &terminal)?;
    }
    // Reestablish exclusive terminal publication durability after lost local sync.
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(terminal_path(journal, reference))?;
        if !file.metadata()?.is_file() {
            return Err(super::invalid_request(
                "unsafe bootstrap terminal durability record",
            ));
        }
        file.sync_all()?;
    }
    std::fs::File::open(journal.root())?.sync_all()?;
    let report = serde_json::json!({"outcome":"started","thread":done.retained.thread,"seat":done.retained.recipient,"pane":done.retained.pane,"launch":done.retained.report,"bootstrap_compound":identity.compound,"child_compound":done.attachment.handoff.compound,"tab":done.attachment.created.tab,"attempt":done.attachment.attempt,"recovery_ref":reference.recovery_ref()});
    let bytes = if output.format == crate::protocol::output::OutputFormat::Json {
        format!("{}\n", serde_json::json!({"handoff":report})).into_bytes()
    } else {
        super::setup::render_text(&report).into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    for path in [
        child_progress_path(journal, reference),
        super::handoff::progress_path(journal, reference),
    ] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    match journal.complete(reference) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    std::fs::File::open(journal.root())?.sync_all()?;
    Ok(current)
}

#[cfg(test)]
#[path = "../../tests/cli/topology_handoff.rs"]
mod tests;
