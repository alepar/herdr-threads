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
#[cfg(test)]
#[path = "../../tests/cli/topology_handoff.rs"]
mod tests;
