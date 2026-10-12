//! Frozen target-seat send composition over the ordinary delivery engine.
use crate::protocol::{commands::DeliveryMode, ids::SeatId, summary::UserIntent};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub target: SeatId,
    pub delivery_mode: DeliveryMode,
    pub body: String,
    pub require_ack: Vec<SeatId>,
    pub deadline_millis: Option<u64>,
    pub relays_user: bool,
    pub user_intent: Option<UserIntent>,
}

use super::{
    RunError,
    journal::{IntentRef, IntentScope, Journal, SemanticMutation},
};
use crate::{
    ports::LocalClient,
    protocol::{
        authority::{CallerClaim, Harness},
        commands::{Command, EnsureDirectThread, SeatInspectQuery, SendMessage},
        handoff::HandoffNamespace,
        ids::{MessageId, OperationId, ThreadId},
        output::{ContinuationContext, OutputFormat, OutputSpec},
        results::{CommandResult, ContinuityStatus, IntentStatus},
        time::Clock,
    },
};
use std::io::{self, Write};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub namespace: HandoffNamespace,
    pub request: Request,
    pub ensure_key: OperationId,
    pub send_key: OperationId,
}
fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail)
}
impl Plan {
    pub fn validate(&self) -> io::Result<()> {
        self.namespace.validate().map_err(invalid)?;
        let r = &self.request;
        if r.body.is_empty()
            || r.body.len() > super::input::MAX_BODY_BYTES
            || r.require_ack.len() > 1
            || r.require_ack.iter().any(|s| s != &r.target)
            || (r.delivery_mode == DeliveryMode::Lazy
                && (!r.require_ack.is_empty() || r.deadline_millis.is_some()))
            || r.deadline_millis == Some(0)
            || self.ensure_key == self.send_key
            || [&self.ensure_key, &self.send_key]
                .iter()
                .any(|k| uuid::Uuid::parse_str(k.as_str()).is_err())
        {
            return Err(invalid("invalid frozen direct send"));
        }
        Ok(())
    }
    pub(crate) fn validate_claim(&self, claim: &CallerClaim) -> io::Result<()> {
        self.validate()?;
        if self.namespace.instance != claim.instance
            || self.request.target == claim.seat
            || (claim.harness != Harness::Human
                && self.request.user_intent.is_some()
                && !self.request.relays_user)
        {
            return Err(invalid("direct send caller or relay mismatch"));
        }
        Ok(())
    }
    fn send(&self, claim: &CallerClaim, thread: ThreadId) -> SendMessage {
        let r = &self.request;
        SendMessage {
            delivery_mode: r.delivery_mode,
            user_intent: r.user_intent,
            thread,
            body: r.body.clone(),
            direct_recipient: Some(r.target.clone()),
            invited_recipients: r.require_ack.clone(),
            deadline_millis: r.deadline_millis,
            operation: self.send_key.clone(),
            claim: claim.clone(),
            relays_user: r.relays_user,
        }
    }
}
pub(crate) fn plan(semantic: &SemanticMutation) -> Option<(&Plan, &CallerClaim)> {
    match semantic {
        SemanticMutation::Frozen { claim, mutation } => match mutation.as_ref() {
            SemanticMutation::DirectSend(p) => Some((p, claim)),
            _ => None,
        },
        _ => None,
    }
}
pub(crate) fn namespace(
    instance: &str,
    context: &ContinuationContext,
) -> io::Result<HandoffNamespace> {
    let runtime = crate::daemon::paths::RuntimeContext::from_environment(
        context.state_dir.as_ref().map(Into::into),
        context.host.as_ref().map(Into::into),
    )?;
    Ok(HandoffNamespace {
        instance: instance.into(),
        state_dir: runtime.state_dir,
        host_endpoint: runtime.host_endpoint,
    })
}
pub(crate) fn check_namespace(p: &Plan, context: &ContinuationContext) -> io::Result<()> {
    if namespace(&p.namespace.instance, context)? != p.namespace {
        return Err(invalid(
            "direct send retry belongs to a different namespace",
        ));
    }
    Ok(())
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    thread: Option<ThreadId>,
    send: Option<SendMessage>,
    message: Option<MessageId>,
}
impl Progress {
    fn validate(&self, p: &Plan, claim: &CallerClaim) -> io::Result<()> {
        match (&self.thread, &self.send) {
            (None, None) if self.message.is_none() => Ok(()),
            (Some(t), Some(s)) if *s == p.send(claim, t.clone()) => Ok(()),
            _ => Err(invalid("contradictory direct send progress")),
        }
    }
}
pub(crate) fn pending(
    journal: &Journal,
    reference: &IntentRef,
) -> io::Result<(Option<ThreadId>, IntentStatus)> {
    let original = journal.load(reference)?;
    let (p, claim) =
        plan(&original.semantic).ok_or_else(|| invalid("direct send needs frozen caller"))?;
    let progress: Progress = super::handoff::load_progress(journal, reference)?;
    progress.validate(p, claim)?;
    let status = if progress.message.is_some() {
        IntentStatus::CommittedAwaitingOutput
    } else if progress.thread.is_some() {
        IntentStatus::Submitted
    } else {
        IntentStatus::Pending
    };
    Ok((progress.thread, status))
}
fn capability<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    lazy: bool,
) -> Result<(), RunError> {
    match client.call(Command::Capabilities, &super::cooperative_budget(clock)) {
        Ok(CommandResult::Capabilities(c))
            if c.capabilities
                .iter()
                .any(|v| v == crate::protocol::capabilities::DIRECT_THREADS_V1)
                && (!lazy
                    || c.capabilities
                        .iter()
                        .any(|v| v == crate::protocol::capabilities::LAZY_SEND)) =>
        {
            Ok(())
        }
        Err(e)
            if !matches!(
                e.code,
                crate::protocol::results::ErrorCode::Unsupported
                    | crate::protocol::results::ErrorCode::InvalidRequest
            ) =>
        {
            Err(e.into())
        }
        _ => Err(super::unsupported(
            "daemon lacks direct-thread/lazy-send capability; upgrade before direct send",
        )),
    }
}
fn inspect<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    seat: &SeatId,
) -> Result<crate::protocol::results::SeatInspection, RunError> {
    let result = client.call(
        Command::SeatInspect(SeatInspectQuery {
            seat: seat.clone(),
            page: crate::protocol::pagination::PageRequest {
                limit: 1,
                ..Default::default()
            },
        }),
        &super::cooperative_budget(clock),
    )?;
    let CommandResult::SeatInspect(v) = result else {
        return Err(super::invalid_request("missing direct seat inspection"));
    };
    if v.summary.seat != *seat
        || v.summary.continuity != ContinuityStatus::Resolved
        || v.mapping.state != ContinuityStatus::Resolved
        || v.summary.target.is_none()
        || v.summary.target != v.mapping.target
        || v.summary.retired_at.is_some()
        || v.retirement.is_some()
        || v.hold.is_some()
    {
        return Err(super::invalid_request(
            "direct seat is not active, resolved and unheld",
        ));
    }
    Ok(v)
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn start<C: LocalClient + ?Sized, W: Write>(
    request: Request,
    claim: CallerClaim,
    journal: &Journal,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let p = Plan {
        namespace: namespace(&claim.instance, &output.context)?,
        request,
        ensure_key: OperationId::new(uuid::Uuid::new_v4().to_string()),
        send_key: OperationId::new(uuid::Uuid::new_v4().to_string()),
    };
    p.validate_claim(&claim)?;
    capability(client, clock, p.request.delivery_mode == DeliveryMode::Lazy)?;
    let caller = inspect(client, clock, &claim.seat)?;
    if caller.summary.generation != claim.binding_generation
        || caller.summary.target.as_ref() != Some(&claim.target)
        || !caller
            .open_binding
            .as_ref()
            .is_some_and(|b| b.target == claim.target && b.harness == claim.harness.as_str())
    {
        return Err(super::invalid_request("direct send caller binding differs"));
    }
    inspect(client, clock, &p.request.target)?;
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    let semantic = SemanticMutation::freeze(SemanticMutation::DirectSend(Box::new(p)), claim)?;
    let reference = journal.record(scope, semantic, clock.utc_now().0)?;
    retry(journal, &reference, client, clock, output, writer)
}
fn recover(
    error: RunError,
    journal: &Journal,
    reference: &IntentRef,
    p: &Plan,
    claim: &CallerClaim,
) -> RunError {
    let mut argv = super::hook::cli_prefix(&ContinuationContext {
        state_dir: Some(p.namespace.state_dir.to_string_lossy().into_owned()),
        host: Some(p.namespace.host_endpoint.to_string_lossy().into_owned()),
    });
    if claim.harness == Harness::Human {
        argv.insert(1, "human".into());
    }
    argv.extend(["retry".into(), reference.recovery_ref()]);
    let command = argv
        .iter()
        .map(|v| {
            shlex::try_quote(v)
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| "<unrepresentable>".into())
        })
        .collect::<Vec<_>>()
        .join(" ");
    let staged = pending(journal, reference)
        .ok()
        .and_then(|v| v.0)
        .map(|t| format!("; staged thread {}", t.as_str()))
        .unwrap_or_default();
    let hint = format!("{staged}; recover with {command}");
    match error {
        RunError::Api(mut e) => {
            e.detail.push_str(&hint);
            e.restart_argv = Some(argv);
            RunError::Api(e)
        }
        other => RunError::Io(io::Error::other(format!("{other:?}{hint}"))),
    }
}
/// Decorate failures from caller/connection reads before executor dispatch.
/// Executor failures already own recovery presentation and never use this seam.
pub(crate) fn pre_executor_error(
    error: RunError,
    parsed: &super::commands::ParsedCli,
    paths: &crate::daemon::paths::InstancePaths,
) -> RunError {
    let super::commands::CliAction::Retry(reference) = &parsed.action else {
        return error;
    };
    let root = paths.instance_dir.join("intents");
    if super::retry::preflight_original_actor(
        &root,
        reference.as_str(),
        parsed.actor,
        &parsed.output.context,
    )
    .is_err()
    {
        return error;
    }
    let Ok(journal) = Journal::read_only(root) else {
        return error;
    };
    let Ok(original) = super::retry::load_original_for_actor(&journal, reference.as_str()) else {
        return error;
    };
    let Some((p, claim)) = plan(&original.semantic) else {
        return error;
    };
    recover(error, &journal, &original.header.reference, p, claim)
}

pub(crate) fn retry<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let original = journal.load(reference)?;
    let (p, claim) =
        plan(&original.semantic).ok_or_else(|| invalid("direct send needs frozen caller"))?;
    p.validate_claim(claim)?;
    check_namespace(p, &output.context)?;
    let _lock = super::handoff::lock(journal, reference)?;
    let result = (|| -> Result<(), RunError> {
        let mut progress: Progress = super::handoff::load_progress(journal, reference)?;
        progress.validate(p, claim)?;
        capability(client, clock, p.request.delivery_mode == DeliveryMode::Lazy)?;
        let ensured = client.call(
            Command::EnsureDirectThread(EnsureDirectThread {
                recipient: p.request.target.clone(),
                operation: p.ensure_key.clone(),
                claim: claim.clone(),
            }),
            &super::cooperative_budget(clock),
        )?;
        let CommandResult::ThreadCreated(thread) = ensured else {
            return Err(super::invalid_request("direct Ensure returned no thread"));
        };
        if progress
            .thread
            .as_ref()
            .is_some_and(|saved| saved != &thread)
        {
            return Err(super::invalid_request(
                "direct Ensure differs from selected thread",
            ));
        }
        let send = p.send(claim, thread.clone());
        progress.thread = Some(thread.clone());
        progress.send = Some(send.clone());
        save(journal, reference, &progress)?;
        let sent = client.call(
            Command::SendMessage(send),
            &super::cooperative_budget(clock),
        )?;
        let CommandResult::MessageSent(message) = sent else {
            return Err(super::invalid_request("direct Send returned no message"));
        };
        if progress
            .message
            .as_ref()
            .is_some_and(|saved| saved != &message)
        {
            return Err(super::invalid_request(
                "direct Send differs from completed message",
            ));
        }
        progress.message = Some(message.clone());
        save(journal, reference, &progress)?;
        let bytes = match output.format {
            OutputFormat::Json => {
                // The ordinary send envelope; `thread` names the pair's channel.
                let mut result = serde_json::to_value(CommandResult::MessageSent(message.clone()))
                    .map_err(io::Error::other)?;
                result["thread"] = serde_json::to_value(&thread).map_err(io::Error::other)?;
                let mut b = serde_json::to_vec(&serde_json::json!({
                    "version": crate::protocol::wire::PROTOCOL_VERSION,
                    "result": result,
                }))
                .map_err(io::Error::other)?;
                b.push(b'\n');
                b
            }
            OutputFormat::Text => {
                format!("thread {} message {}\n", thread.as_str(), message.as_str()).into_bytes()
            }
        };
        if bytes.len() > crate::protocol::pagination::MAX_PAGE_BYTES as usize {
            return Err(super::invalid_request("direct output exceeds budget"));
        }
        writer.write_all(&bytes)?;
        writer.flush()?;
        std::fs::remove_file(super::handoff::progress_path(journal, reference))?;
        std::fs::File::open(journal.root())?.sync_all()?;
        journal.complete(reference)?;
        Ok(())
    })();
    result.map_err(|e| recover(e, journal, reference, p, claim))
}
fn save(journal: &Journal, reference: &IntentRef, progress: &Progress) -> io::Result<()> {
    if serde_json::to_vec(progress)?.len() > 4 * 1024 * 1024 {
        return Err(invalid("oversized direct progress"));
    }
    super::handoff::save_progress(journal, reference, progress)
}
