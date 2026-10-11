//! Existing-peer delivery with an explicit frozen-origin production envelope.
//! Seat reads select and conservatively refuse; they cannot authorize a live
//! effect. Activation must supply daemon-selected namespace and deciding current
//! recipient guards in addition to the legacy caller/membership fences.
use super::journal::PendingIntent;
use super::{
    RunError, handoff,
    journal::{DeliveryPlan, IntentRef, IntentScope, Journal, SemanticMutation},
    topology_handoff::Request,
};
use crate::protocol::{
    handoff::HandoffResult,
    output::{OutputFormat, OutputSpec},
};
use crate::{
    ports::LocalClient,
    protocol::{
        authority::{CallerClaim, CallerRole},
        commands::{Command, ResolveThreadQuery, SeatInspectQuery},
        handoff::{
            HandoffChannel, HandoffIdentity, HandoffKeys, HandoffNamespace, HandoffPayload,
            HandoffState,
        },
        ids::{OperationId, SeatId},
        pagination::PageRequest,
        results::{ApiError, CommandResult, ContinuityStatus, ErrorCode, SeatInspection},
        time::Clock,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
};
pub struct Preparation<'a, C: LocalClient + ?Sized> {
    pub claim: &'a CallerClaim,
    pub namespace: &'a HandoffNamespace,
    pub topology: &'a crate::host::observation::HostTopology,
    pub client: &'a C,
    pub clock: &'a dyn Clock,
}
#[allow(clippy::too_many_arguments)]
pub fn run_automatic_existing<C: LocalClient + ?Sized, W: Write>(
    request: &Request,
    inputs: Preparation<'_, C>,
    journal: &Journal,
    actor: super::actor_route::InvocationActor,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    require_automatic_capability(inputs.client, inputs.clock)?;
    let claim = inputs.claim.clone();
    let namespace = inputs.namespace.clone();
    let clock = inputs.clock;
    let client = inputs.client;
    let plan = prepare_automatic(request, inputs)?;
    let original = crate::protocol::handoff::AutomaticOriginal::Delivery(Box::new(plan));
    let envelope = automatic_envelope(&claim, original.clone())?;
    match client.call(
        Command::AutomaticHandoffDelivery(Box::new(envelope)),
        &super::cooperative_budget(clock),
    )? {
        CommandResult::SeatResolved(seat) if seat == original.delivery().recipient => {}
        _ => {
            return Err(super::invalid_request(
                "unexpected automatic preparation result",
            ));
        }
    }
    let reference = publish_automatic(journal, original, claim, clock)?;
    retry_automatic_to_writer(
        journal, &reference, actor, &namespace, client, None, clock, output, writer,
    )?;
    Ok(())
}

pub(crate) fn require_automatic_capability<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    match client.call(Command::Capabilities, &super::cooperative_budget(clock)) {
        Ok(CommandResult::Capabilities(list))
            if list.capabilities.iter().any(|capability| {
                capability == crate::protocol::capabilities::AUTOMATIC_HANDOFF_V1
            }) =>
        {
            Ok(())
        }
        Ok(_) => Err(super::unsupported(
            "daemon lacks automatic handoff capability",
        )),
        Err(error) if error.code == ErrorCode::Unsupported => Err(super::unsupported(
            "daemon lacks automatic handoff capability",
        )),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn automatic_envelope(
    claim: &CallerClaim,
    original: crate::protocol::handoff::AutomaticOriginal,
) -> Result<crate::protocol::handoff::AutomaticDeliveryMutation, RunError> {
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    crate::protocol::handoff::AutomaticDeliveryMutation::new(scope, claim.clone(), original)
        .map_err(super::invalid_request)
}

pub(crate) fn publish_automatic(
    journal: &Journal,
    original: crate::protocol::handoff::AutomaticOriginal,
    claim: CallerClaim,
    clock: &dyn Clock,
) -> Result<IntentRef, RunError> {
    automatic_envelope(&claim, original.clone())?;
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    Ok(journal.record(scope, original.semantic(claim), clock.utc_now().0)?)
}

pub(crate) fn prepare_automatic<C: LocalClient + ?Sized>(
    request: &Request,
    inputs: Preparation<'_, C>,
) -> Result<crate::protocol::handoff::AutomaticDeliveryPlan, RunError> {
    let fail = super::invalid_request;
    if !request.automatic
        || !request.existing
        || request.new_tab.is_some()
        || request.cwd.is_some()
        || request.kind.is_some()
        || request.binary.is_some()
        || request.name.is_some()
        || !request.argv.is_empty()
        || request.seat.is_some() == request.selector.pane.is_some()
        || (request.seat.is_some()
            && (request.selector.space.is_some() || request.selector.tab.is_some()))
        || request.thread.is_some()
        || request.thread_name.is_some()
        || request.topic.is_some()
        || request.goal.is_some()
    {
        return Err(fail("invalid automatic existing-peer request"));
    }
    inputs.namespace.validate().map_err(fail)?;
    if inputs.claim.role != CallerRole::TopLevel
        || inputs.claim.instance != inputs.namespace.instance
    {
        return Err(fail(
            "automatic delivery requires original top-level instance claim",
        ));
    }
    let (recipient, target) = if let Some(seat) = &request.seat {
        (SeatId::new(seat.clone()), None)
    } else {
        let target = super::panes::resolve_selector(
            &request.selector,
            inputs.topology,
            Some(inputs.claim.target.as_str()),
        )?;
        let found = super::collect_pane_seats(&target, |command| {
            inputs
                .client
                .call(command, &super::cooperative_budget(inputs.clock))
        })
        .map_err(|error| match error {
            super::PaneSeatsError::Api(error) => RunError::from(error),
            super::PaneSeatsError::Unexpected => fail("unexpected canonical seats result"),
        })?;
        let [seat] = found.resolved.as_slice() else {
            return Err(fail(
                "automatic delivery pane needs exactly one resolved seat",
            ));
        };
        (seat.seat.clone(), Some(target))
    };
    let found = inspect(inputs.client, inputs.clock, &recipient)?;
    if target.is_some() && target != found.mapping.target {
        return Err(fail(
            "automatic delivery pane mapping changed during selection",
        ));
    }
    let payload = crate::protocol::handoff::AutomaticHandoffPayload::new(
        inputs.namespace.clone(),
        OperationId::new(uuid::Uuid::new_v4().to_string()),
        request.body.clone(),
    )
    .map_err(fail)?;
    crate::protocol::handoff::AutomaticDeliveryPlan::new(payload, recipient).map_err(fail)
}
fn inspect<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    seat: &SeatId,
) -> Result<SeatInspection, RunError> {
    let CommandResult::SeatInspect(found) = client.call(
        Command::SeatInspect(SeatInspectQuery {
            seat: seat.clone(),
            page: PageRequest::default(),
        }),
        &super::cooperative_budget(clock),
    )?
    else {
        return Err(super::invalid_request("unexpected seat inspection result"));
    };
    if &found.summary.seat != seat
        || found.summary.continuity != ContinuityStatus::Resolved
        || found.mapping.state != ContinuityStatus::Resolved
        || found.summary.retired_at.is_some()
        || found.retirement.is_some()
        || found.hold.is_some()
        || found.mapping.target.is_none()
        || found.mapping.target != found.summary.target
    {
        return Err(super::invalid_request(
            "delivery needs an existing resolved, nonretired, unheld seat",
        ));
    }
    Ok(found)
}
/// Pure preparation plus canonical reads. No launch option environment is read.
pub fn prepare<C: LocalClient + ?Sized>(
    request: &Request,
    inputs: Preparation<'_, C>,
) -> Result<DeliveryPlan, RunError> {
    if request.automatic {
        return Err(super::unsupported(
            "automatic delivery producer unavailable",
        ));
    }
    let fail = super::invalid_request;
    if let Some(policy) = &request.policy {
        policy.validate().map_err(super::invalid_request)?;
        super::require_handoff_capabilities(inputs.client, inputs.clock, Some(policy))?;
    }
    if !request.existing
        || request.new_tab.is_some()
        || request.cwd.is_some()
        || request.kind.is_some()
        || request.binary.is_some()
        || request.name.is_some()
        || !request.argv.is_empty()
        || request.seat.is_some() == request.selector.pane.is_some()
        || (request.seat.is_some()
            && (request.selector.space.is_some() || request.selector.tab.is_some()))
        || (request.thread.is_some()
            && (request.thread_name.is_some() || request.topic.is_some() || request.goal.is_some()))
    {
        return Err(fail("invalid existing-peer delivery request"));
    }
    inputs.namespace.validate().map_err(fail)?;
    if inputs.claim.role != CallerRole::TopLevel
        || inputs.claim.instance != inputs.namespace.instance
    {
        return Err(fail(
            "delivery requires the original top-level instance claim",
        ));
    }
    let (recipient, target) = if let Some(seat) = &request.seat {
        (SeatId::new(seat.clone()), None)
    } else {
        let target = super::panes::resolve_selector(
            &request.selector,
            inputs.topology,
            Some(inputs.claim.target.as_str()),
        )?;
        let found = super::collect_pane_seats(&target, |command| {
            inputs
                .client
                .call(command, &super::cooperative_budget(inputs.clock))
        })
        .map_err(|error| match error {
            super::PaneSeatsError::Api(error) => RunError::from(error),
            super::PaneSeatsError::Unexpected => fail("unexpected canonical seats result"),
        })?;
        let [seat] = found.resolved.as_slice() else {
            return Err(fail(
                "delivery pane must select exactly one existing resolved seat",
            ));
        };
        (seat.seat.clone(), Some(target))
    };
    let found = inspect(inputs.client, inputs.clock, &recipient)?;
    if target.is_some() && target != found.mapping.target {
        return Err(fail("delivery pane mapping changed during selection"));
    }
    let channel = if let Some(selector) = &request.thread {
        let CommandResult::ThreadResolved(thread) = inputs.client.call(
            Command::ResolveThread(ResolveThreadQuery {
                selector: selector.clone(),
                caller: Some(inputs.claim.seat.clone()),
                caller_target: Some(inputs.claim.target.clone()),
            }),
            &super::cooperative_budget(inputs.clock),
        )?
        else {
            return Err(fail("unexpected thread resolution result"));
        };
        handoff::membership(inputs.client, &thread, inputs.claim, inputs.clock)?;
        HandoffChannel::Existing { thread }
    } else {
        let topic = request
            .topic
            .clone()
            .unwrap_or_else(|| format!("Handoff to {}", recipient.as_str()));
        HandoffChannel::New {
            name: request.thread_name.clone(),
            goal: request.goal.clone().unwrap_or_else(|| topic.clone()),
            topic,
        }
    };
    let compound = OperationId::new(uuid::Uuid::new_v4().to_string());
    let child = |phase: &str| {
        use sha2::{Digest, Sha256};
        OperationId::new(format!(
            "delivery-{:x}",
            Sha256::digest(format!("{}\0{phase}", compound.as_str()).as_bytes())
        ))
    };
    let plan = DeliveryPlan {
        version: 1,
        recipient,
        payload: HandoffPayload {
            policy: request.policy.clone(),
            namespace: inputs.namespace.clone(),
            channel,
            body: request.body.clone(),
            keys: HandoffKeys {
                compound: compound.clone(),
                begin: child("begin"),
                create: child("create"),
                invite: child("invite"),
                send: child("send"),
                complete: child("complete"),
            },
        },
    };
    plan.validate()?;
    Ok(plan)
}
/// Production effect wrapper keeps the original immutable origin on every phase.
pub(crate) struct FrozenDeliveryClient<'a, C: LocalClient + ?Sized> {
    client: &'a C,
    envelope: crate::protocol::handoff::DeliveryMutation,
}
impl<'a, C: LocalClient + ?Sized> FrozenDeliveryClient<'a, C> {
    pub(crate) fn new(client: &'a C, pending: &PendingIntent) -> Result<Self, RunError> {
        let plan = delivery_plan(pending)?.clone();
        let claim = pending
            .semantic
            .frozen_claim()
            .ok_or_else(|| super::invalid_request("delivery needs frozen caller"))?
            .clone();
        Self::from_plan(
            client,
            pending.header.scope.clone(),
            claim,
            pending.header.semantic_digest.clone(),
            plan,
        )
    }
    pub(crate) fn from_plan(
        client: &'a C,
        scope: IntentScope,
        claim: CallerClaim,
        digest: String,
        plan: DeliveryPlan,
    ) -> Result<Self, RunError> {
        let keys = plan.payload.keys.clone();
        let identity = HandoffIdentity {
            compound: keys.compound.clone(),
            digest: digest.clone(),
            claim: claim.clone(),
            thread: plan.payload.channel.thread().cloned(),
            recipient: plan.recipient.clone(),
            create_key: keys.create.clone(),
            invite_key: keys.invite.clone(),
            send_key: keys.send.clone(),
        };
        let envelope = crate::protocol::handoff::DeliveryMutation {
            scope,
            claim,
            digest,
            plan,
            action: crate::protocol::handoff::DeliveryAction::Begin(
                crate::protocol::handoff::HandoffMutation {
                    identity,
                    operation: keys.begin.clone(),
                },
            ),
        };
        envelope.validate().map_err(super::invalid_request)?;
        Ok(Self { client, envelope })
    }
    pub(crate) fn query(
        &self,
        prepare: bool,
        clock: &dyn Clock,
    ) -> Result<CommandResult, RunError> {
        let mut envelope = self.envelope.clone();
        envelope.action = if prepare {
            crate::protocol::handoff::DeliveryAction::Prepare(envelope.identity())
        } else {
            crate::protocol::handoff::DeliveryAction::Status(envelope.identity())
        };
        Ok(self.client.call(
            Command::HandoffDelivery(Box::new(envelope)),
            &super::cooperative_budget(clock),
        )?)
    }
    fn wrap(&self, command: Command) -> Result<Command, ApiError> {
        use crate::protocol::handoff::DeliveryAction;
        let action = match command {
            Command::HandoffDelivery(envelope) => {
                let mut expected = self.envelope.clone();
                expected.action =
                    crate::protocol::handoff::DeliveryAction::Status(expected.identity());
                if *envelope != expected {
                    return Err(ApiError::invalid_request(
                        "delivery status differs from frozen envelope",
                    ));
                }
                envelope.validate().map_err(ApiError::invalid_request)?;
                return Ok(Command::HandoffDelivery(envelope));
            }
            Command::BeginHandoff(v) => DeliveryAction::Begin(v),
            Command::CreateThread(v) => DeliveryAction::Create(v),
            Command::Invite(v) => DeliveryAction::Invite(v),
            Command::SendMessage(v) => DeliveryAction::Send(v),
            Command::CompleteHandoff(v) => DeliveryAction::Complete(v),
            read @ (Command::Capabilities | Command::SeatInspect(_) | Command::Participants(_)) => {
                return Ok(read);
            }
            _ => {
                return Err(ApiError::invalid_request(
                    "delivery executor attempted unrelated command",
                ));
            }
        };
        let mut envelope = self.envelope.clone();
        envelope.action = action;
        envelope.validate().map_err(ApiError::invalid_request)?;
        Ok(Command::HandoffDelivery(Box::new(envelope)))
    }
}
impl<C: LocalClient + ?Sized> LocalClient for FrozenDeliveryClient<'_, C> {
    fn call(
        &self,
        command: Command,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.client.call(self.wrap(command)?, budget)
    }
    fn call_with_output(
        &self,
        command: Command,
        output: &OutputSpec,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.client
            .call_with_output(self.wrap(command)?, output, budget)
    }
}

/// Publish the full frozen semantic before Begin or any staged-work effect.
pub fn publish(
    journal: &Journal,
    plan: DeliveryPlan,
    claim: CallerClaim,
    clock: &dyn Clock,
) -> Result<IntentRef, RunError> {
    plan.validate()?;
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    Ok(journal.record(
        scope,
        SemanticMutation::freeze(SemanticMutation::HandoffDelivery(Box::new(plan)), claim)?,
        clock.utc_now().0,
    )?)
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    staged: handoff::StagedWork,
    report: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    report_digest: Option<String>,
}
// Keep the legacy shared staged-work schema and reader unchanged. Delivery
// checks the raw frame before unknown nested fields can be discarded by serde.
fn load_delivery_progress(journal: &Journal, reference: &IntentRef) -> io::Result<Progress> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(handoff::progress_path(journal, reference))
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Progress::default()),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("unsafe delivery progress"));
    }
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(io::Error::other("oversized delivery progress"));
    }
    let progress: Progress = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if serde_json::from_slice::<serde_json::Value>(&bytes).map_err(io::Error::other)?
        != serde_json::to_value(&progress).map_err(io::Error::other)?
    {
        return Err(io::Error::other("unexpected delivery progress fields"));
    }
    Ok(progress)
}
fn retained_digest(value: &impl Serialize) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
/// Validate the actual producer's retained report; never fill missing identifiers.
fn retained_report(
    reference: &IntentRef,
    plan: &DeliveryPlan,
    current: &crate::protocol::handoff::HandoffResult,
    progress: &Progress,
) -> Result<serde_json::Value, RunError> {
    let fail =
        || super::invalid_request("completed delivery needs its exact retained staged-work report");
    let staged = &progress.staged;
    let report = progress.report.as_ref().ok_or_else(fail)?;
    let participation = report["participation"].as_str().ok_or_else(fail)?;
    if progress.report_digest.as_deref() != Some(retained_digest(report)?.as_str()) {
        return Err(fail());
    }
    if current.state != HandoffState::Completed
        || current.compound != plan.payload.keys.compound
        || current.thread.is_none()
        || current.thread != staged.thread
        || plan
            .payload
            .channel
            .thread()
            .is_some_and(|thread| Some(thread) != staged.thread.as_ref())
        || !matches!(&staged.message, Some(CommandResult::MessageSent(id)) if !id.as_str().is_empty())
        || !matches!(
            participation,
            "joined" | "invited_pending" | "staged_unbound"
        )
        || match &staged.invitation {
            Some(CommandResult::Invitation(id)) => {
                !staged.invitation_attempted || id.as_str().is_empty()
            }
            Some(CommandResult::AlreadyJoined(joined)) => {
                Some(&joined.thread) != staged.thread.as_ref() || joined.seat != plan.recipient
            }
            None => true,
            _ => true,
        }
    {
        return Err(fail());
    }
    let expected = serde_json::json!({"compound":plan.payload.keys.compound,"thread":staged.thread,"recipient":plan.recipient,"participation":participation,"outcome":"staged","invitation":staged.invitation,"message":staged.message,"recovery_ref":reference.recovery_ref()});
    if report != &expected {
        return Err(fail());
    }
    Ok(report.clone())
}
/// Internal staging entry for final activation and delivery retry integration.
/// Leaves publication/progress intact for terminal presentation/cleanup.
/// Existing Begin/Complete validate original caller A2 and channel membership;
/// they do not carry namespace or a deciding recipient-mapping guard.
pub fn execute<C: LocalClient + ?Sized>(
    journal: &Journal,
    reference: &IntentRef,
    client: &C,
    clock: &dyn Clock,
) -> Result<serde_json::Value, RunError> {
    let _lock = handoff::lock(journal, reference)?;
    execute_locked(
        journal,
        reference,
        journal.load(reference)?,
        client,
        clock,
        &mut None,
    )
    .map(|(report, _)| report)
}
/// Transient, invocation-local observations of admitted unfinished delivery.
/// Captured only after original/scope validation, canonical Begin and a valid
/// retained progress read; never persisted and never used as authority.
struct DeliveryObservation {
    plan: DeliveryPlan,
    caller_target: crate::protocol::ids::HostTargetId,
    phase: &'static str,
    staged: handoff::StagedWork,
    /// None: no canonical status was observed by this invocation (unknown).
    last_observed: Option<HandoffResult>,
}
/// Begin transport failures and lost replies are indistinguishable to the
/// client after publication; daemon refusals (authority, scope, payload,
/// validation) are not uncertain and stay silent.
fn begin_uncertain(error: &RunError) -> bool {
    use crate::protocol::results::ErrorCode;
    match error {
        RunError::Io(_) => true,
        RunError::Api(e) => matches!(
            e.code,
            ErrorCode::UnknownOutcome
                | ErrorCode::HostUnavailable
                | ErrorCode::DeadlineExceeded
                | ErrorCode::Cancelled
                | ErrorCode::DaemonBootChanged
                | ErrorCode::ServiceBusy
                | ErrorCode::StoreBusy
        ),
        _ => false,
    }
}
fn execute_locked<C: LocalClient + ?Sized>(
    journal: &Journal,
    reference: &IntentRef,
    pending: PendingIntent,
    client: &C,
    clock: &dyn Clock,
    observed: &mut Option<DeliveryObservation>,
) -> Result<(serde_json::Value, HandoffResult), RunError> {
    let SemanticMutation::Frozen { claim, mutation } = pending.semantic else {
        return Err(super::invalid_request(
            "delivery needs a frozen original caller",
        ));
    };
    let SemanticMutation::HandoffDelivery(plan) = *mutation else {
        return Err(super::invalid_request("not a delivery reference"));
    };
    plan.validate()?;
    if pending.header.scope
        != (IntentScope::Cooperative {
            instance: claim.instance.clone(),
            seat: claim.seat.clone(),
        })
        || claim.instance != plan.payload.namespace.instance
        || claim.role != CallerRole::TopLevel
    {
        return Err(super::invalid_request("delivery original scope mismatch"));
    }
    // Read the exact canonical frozen identity before deciding whether any new
    // operation is needed. Local staged reports cannot establish completion.
    let status_client = FrozenDeliveryClient::from_plan(
        client,
        pending.header.scope.clone(),
        claim.clone(),
        pending.header.semantic_digest.clone(),
        (*plan).clone(),
    )?;
    let status = match status_client.query(false, clock) {
        Ok(CommandResult::Handoff(current)) => Some(current),
        Err(RunError::Api(error)) if error.code == ErrorCode::NotFound => None,
        Err(error) => return Err(error),
        Ok(_) => return Err(super::invalid_request("unexpected delivery status result")),
    };
    let completed = status.filter(|current| current.state == HandoffState::Completed);
    if completed.is_none() {
        super::require_handoff_capabilities(client, clock, plan.payload.policy.as_ref())?;
    }
    let keys = &plan.payload.keys;
    let identity = HandoffIdentity {
        compound: keys.compound.clone(),
        digest: pending.header.semantic_digest,
        claim: claim.clone(),
        thread: plan.payload.channel.thread().cloned(),
        recipient: plan.recipient.clone(),
        create_key: keys.create.clone(),
        invite_key: keys.invite.clone(),
        send_key: keys.send.clone(),
    };
    // Validated original after publication: canonical status is unknown until
    // Begin answers. Non-uncertain refusals clear this before returning.
    *observed = Some(DeliveryObservation {
        plan: (*plan).clone(),
        caller_target: claim.target.clone(),
        phase: "begin",
        staged: Default::default(),
        last_observed: None,
    });
    let current = match completed.map(Ok).unwrap_or_else(|| {
        handoff::keyed_fence(client, clock, &identity, keys.begin.clone(), false)
    }) {
        Ok(current) => current,
        Err(error) => {
            if !begin_uncertain(&error) {
                *observed = None;
            }
            return Err(error);
        }
    };
    let mut progress: Progress = match load_delivery_progress(journal, reference) {
        Ok(progress) => progress,
        Err(error) => {
            // Malformed retained progress (Other) is a silent refusal; an
            // ordinary I/O failure leaves only canonical status observed.
            match observed {
                Some(observed) if error.kind() != io::ErrorKind::Other => {
                    observed.phase = "progress";
                    observed.last_observed = Some(current.clone());
                }
                _ => *observed = None,
            }
            return Err(error.into());
        }
    };
    if current.state == HandoffState::Completed {
        return match retained_report(reference, &plan, &current, &progress) {
            Ok(report) => {
                if let Some(observed) = observed {
                    observed.phase = "terminal";
                    observed.staged = progress.staged.clone();
                    observed.last_observed = Some(current.clone());
                }
                Ok((report, current))
            }
            Err(error) => {
                *observed = None;
                Err(error)
            }
        };
    }
    let mut staged = progress.staged.clone();
    if let Some(observed) = observed.as_mut() {
        observed.phase = "recipient";
        observed.staged = staged.clone();
        observed.last_observed = Some(current.clone());
    }
    let observe = |observed: &mut Option<DeliveryObservation>,
                   phase: &'static str,
                   staged: &handoff::StagedWork| {
        if let Some(observed) = observed {
            observed.phase = phase;
            observed.staged = staged.clone();
        }
    };
    let recipient = inspect(client, clock, &plan.recipient)?;
    let mut phase = "create";
    let call = |semantic: SemanticMutation, key: &OperationId| -> Result<CommandResult, RunError> {
        Ok(client.call(
            semantic.to_command(key.clone(), Some(claim.clone()))?,
            &super::cooperative_budget(clock),
        )?)
    };
    let staging = handoff::stage_work(
        handoff::Staging {
            policy: plan.payload.policy.as_ref(),
            channel: &plan.payload.channel,
            body: &plan.payload.body,
            recipient: &plan.recipient,
            create_key: &keys.create,
            invite_key: &keys.invite,
            send_key: &keys.send,
            skip_joined: true,
        },
        &mut staged,
        &mut phase,
        &call,
        &mut |staged| {
            progress.staged = staged.clone();
            handoff::save_progress(journal, reference, &progress).map_err(RunError::from)
        },
        client,
        clock,
    );
    // Typed returned child results stay observations even when their local
    // save failed; the presenter never claims local durability for them.
    observe(observed, phase, &staged);
    staging?;
    observe(observed, "participation", &staged);
    let participation = if recipient.open_binding.is_none() {
        "staged_unbound"
    } else if handoff::recipient_joined(
        client,
        clock,
        staged.thread.as_ref().unwrap(),
        &plan.recipient,
    )? {
        "joined"
    } else {
        "invited_pending"
    };
    let report = serde_json::json!({"compound":keys.compound,"thread":staged.thread,"recipient":plan.recipient,"participation":participation,"outcome":"staged","invitation":staged.invitation,"message":staged.message,"recovery_ref":reference.recovery_ref()});
    observe(observed, "report", &staged);
    progress.report = Some(report.clone());
    progress.report_digest = Some(retained_digest(&report)?);
    handoff::save_progress(journal, reference, &progress)?;
    // Exact retained terminal capacity is known before canonical completion.
    // Preserve the original bytes rather than rebuilding its historical JSON.
    let preview = Terminal {
        version: 1,
        original: String::from_utf8(journal.snapshot_delivery_origin(reference)?)
            .map_err(io::Error::other)?,
        completed: HandoffResult {
            compound: identity.compound.clone(),
            thread: staged.thread.clone(),
            state: HandoffState::Completed,
        },
        progress_digest: retained_digest(&progress)?,
        progress,
    };
    terminal_bytes(&preview)?;
    observe(observed, "complete", &staged);
    let completed = handoff::keyed_fence(client, clock, &identity, keys.complete.clone(), true)?;
    retained_report(reference, &plan, &completed, &preview.progress)?;
    observe(observed, "terminal", &staged);
    if let Some(observed) = observed.as_mut() {
        observed.last_observed = Some(completed.clone());
    }
    Ok((report, completed))
}
/// Private cleanup recovery evidence. Retained after cleanup; never live authority.
/// The original is the exact historical header/newline/Frozen semantic bytes.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Terminal {
    version: u32,
    original: String,
    completed: HandoffResult,
    progress: Progress,
    progress_digest: String,
}
// A maximum body consumes 393216 JSON bytes in the original and 458752
// when that exact JSON is nested as a string. Keep bounded report headroom.
pub(crate) const MAX_DELIVERY_TERMINAL_BYTES: usize = 1024 * 1024;
fn terminal_path(journal: &Journal, reference: &IntentRef) -> std::path::PathBuf {
    journal.root().join(format!(
        "delivery-{:020}-{}.terminal",
        reference.ordinal,
        reference.operation.as_str()
    ))
}
fn delivery_plan(pending: &PendingIntent) -> Result<&DeliveryPlan, RunError> {
    let SemanticMutation::Frozen { claim, mutation } = &pending.semantic else {
        return Err(super::invalid_request(
            "delivery needs original frozen caller",
        ));
    };
    let SemanticMutation::HandoffDelivery(plan) = mutation.as_ref() else {
        return Err(super::invalid_request("not a delivery origin"));
    };
    plan.validate()?;
    if pending.header.scope
        != (IntentScope::Cooperative {
            instance: claim.instance.clone(),
            seat: claim.seat.clone(),
        })
        || claim.instance != plan.payload.namespace.instance
        || claim.role != CallerRole::TopLevel
    {
        return Err(super::invalid_request("delivery original scope mismatch"));
    }
    Ok(plan)
}
fn read_terminal(journal: &Journal, reference: &IntentRef) -> Result<Option<Terminal>, RunError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(terminal_path(journal, reference))
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_DELIVERY_TERMINAL_BYTES as u64 {
        return Err(super::invalid_request(
            "unsafe or oversized delivery terminal record",
        ));
    }
    let mut bytes = Vec::new();
    file.take((MAX_DELIVERY_TERMINAL_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_DELIVERY_TERMINAL_BYTES {
        return Err(super::invalid_request("oversized delivery terminal record"));
    }
    let terminal: Terminal = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if serde_json::from_slice::<serde_json::Value>(&bytes).map_err(io::Error::other)?
        != serde_json::to_value(&terminal).map_err(io::Error::other)?
    {
        return Err(super::invalid_request(
            "unexpected delivery terminal fields",
        ));
    }
    if terminal.progress_digest != retained_digest(&terminal.progress)? {
        return Err(super::invalid_request(
            "delivery terminal progress digest mismatch",
        ));
    }
    if terminal.version != 1 {
        return Err(super::invalid_request(
            "unsupported delivery terminal record",
        ));
    }
    let pending = Journal::decode_delivery_origin(reference, terminal.original.as_bytes())?;
    let plan = delivery_plan(&pending)?;
    retained_report(reference, plan, &terminal.completed, &terminal.progress)?;
    // Existing files must agree. An absent file is expected during interrupted cleanup.
    match journal.snapshot_delivery_origin(reference) {
        Ok(original) if original != terminal.original.as_bytes() => {
            return Err(super::invalid_request(
                "delivery terminal origin contradiction",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if handoff::progress_path(journal, reference).try_exists()? {
        let progress: Progress = load_delivery_progress(journal, reference)?;
        if serde_json::to_value(progress).map_err(io::Error::other)?
            != serde_json::to_value(&terminal.progress).map_err(io::Error::other)?
        {
            return Err(super::invalid_request(
                "delivery terminal progress contradiction",
            ));
        }
    }
    Ok(Some(terminal))
}
fn terminal_bytes(terminal: &Terminal) -> Result<Vec<u8>, RunError> {
    let bytes = serde_json::to_vec(terminal).map_err(io::Error::other)?;
    if bytes.len() > MAX_DELIVERY_TERMINAL_BYTES {
        return Err(super::invalid_request("oversized delivery terminal record"));
    }
    Ok(bytes)
}
fn save_terminal(
    journal: &Journal,
    reference: &IntentRef,
    terminal: &Terminal,
) -> Result<(), RunError> {
    let bytes = terminal_bytes(terminal)?;
    let temp = journal
        .root()
        .join(format!(".delivery-terminal-{}", uuid::Uuid::new_v4()));
    let save = || -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        // Exclusive publication: never replace an existing historical terminal record.
        fs::hard_link(&temp, terminal_path(journal, reference))?;
        File::open(journal.root())?.sync_all()
    };
    let result = save();
    let cleanup = fs::remove_file(&temp);
    result?;
    cleanup?;
    File::open(journal.root())?.sync_all()?;
    Ok(())
}
/// Read-only original selection for the shared original-actor gate at activation.
/// Caller must run that classifier BEFORE connecting or invoking retry/presentation.
pub fn load_original(journal: &Journal, reference: &IntentRef) -> Result<PendingIntent, RunError> {
    if let Some(terminal) = read_terminal(journal, reference)? {
        return Ok(Journal::decode_delivery_origin(
            reference,
            terminal.original.as_bytes(),
        )?);
    }
    let bytes = journal.snapshot_delivery_origin(reference)?;
    let pending = Journal::decode_delivery_origin(reference, &bytes)?;
    delivery_plan(&pending)?;
    Ok(pending)
}
/// Resolve a retained delivery reference without effects. Historical journal
/// and terminal names select the same exact reference or refuse ambiguity.
/// All bytes are read through the hardened bounded delivery reader.
pub fn resolve_recovery_ref(journal: &Journal, value: &str) -> Result<IntentRef, RunError> {
    let ordinal: u64 = value
        .strip_prefix("local:")
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| super::invalid_request("invalid delivery reference"))?;
    if value != format!("local:{ordinal}") {
        return Err(super::invalid_request("noncanonical delivery reference"));
    }
    let prefix = format!("delivery-{ordinal:020}-");
    let intent_prefix = format!("{ordinal:020}-");
    let mut found = None;
    for entry in fs::read_dir(journal.root())? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if let Some(operation) = name
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(".terminal"))
            .or_else(|| {
                name.strip_prefix(&intent_prefix)
                    .and_then(|s| s.strip_suffix(".intent"))
            })
        {
            let reference = IntentRef {
                ordinal,
                operation: OperationId::parse(operation.to_owned())
                    .map_err(super::invalid_request)?,
            };
            load_original(journal, &reference)?;
            if found
                .as_ref()
                .is_some_and(|previous| previous != &reference)
            {
                return Err(super::invalid_request("ambiguous delivery reference"));
            }
            found = Some(reference);
        }
    }
    found.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "delivery reference not found").into()
    })
}
/// Internal guarded delivery retry. Activation must first classify the saved
/// original actor and enforce daemon-selected namespace/current-recipient guards.
#[allow(clippy::too_many_arguments)]
pub fn retry_to_writer<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    namespace: &HandoffNamespace,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<serde_json::Value, RunError> {
    let _lock = handoff::lock(journal, reference)?;
    let retained = read_terminal(journal, reference)?;
    let pending = if let Some(terminal) = &retained {
        Journal::decode_delivery_origin(reference, terminal.original.as_bytes())?
    } else {
        load_original(journal, reference)?
    };
    let plan = delivery_plan(&pending)?;
    namespace.validate().map_err(super::invalid_request)?;
    if namespace.instance != plan.payload.namespace.instance
        || namespace.state_dir.as_os_str() != plan.payload.namespace.state_dir.as_os_str()
        || namespace.host_endpoint.as_os_str() != plan.payload.namespace.host_endpoint.as_os_str()
    {
        return Err(super::invalid_request(
            "delivery current namespace mismatch",
        ));
    }
    let terminal = if let Some(terminal) = retained {
        let claim = pending.semantic.frozen_claim().unwrap().clone();
        let frozen = FrozenDeliveryClient::from_plan(
            client,
            pending.header.scope.clone(),
            claim,
            pending.header.semantic_digest.clone(),
            plan.clone(),
        )?;
        let CommandResult::Handoff(canonical) = frozen.query(false, clock)? else {
            return Err(super::invalid_request("unexpected delivery status result"));
        };
        if canonical != terminal.completed {
            return Err(super::invalid_request(
                "delivery terminal disagrees with canonical completion",
            ));
        }
        // Publication may have left a visible hard link after directory sync
        // failed. Reestablish durability before presentation or any unlink;
        // the later cleanup sync cannot prove this earlier boundary.
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(terminal_path(journal, reference))?
            .sync_all()?;
        File::open(journal.root())?.sync_all()?;
        terminal
    } else {
        let original = journal.snapshot_delivery_origin(reference)?;
        let mut observed = None;
        let (_, completed) =
            match execute_locked(journal, reference, pending, client, clock, &mut observed) {
                Ok(done) => done,
                Err(error) => {
                    if super::topology_handoff::reportable_failure(&error)
                        && let Some(observed) = &observed
                    {
                        write_pending(reference, observed, output, writer)?;
                    }
                    return Err(error);
                }
            };
        // Canonical completion was observed; a local terminal failure is
        // reported (malformed reloaded progress stays a silent refusal).
        let saved = (|| -> Result<Terminal, (RunError, bool)> {
            let progress: Progress = load_delivery_progress(journal, reference).map_err(|e| {
                let report = e.kind() != io::ErrorKind::Other;
                (RunError::from(e), report)
            })?;
            let progress_digest = retained_digest(&progress).map_err(|e| (e.into(), true))?;
            let terminal = Terminal {
                version: 1,
                original: String::from_utf8(original)
                    .map_err(|e| (io::Error::other(e).into(), true))?,
                completed,
                progress,
                progress_digest,
            };
            save_terminal(journal, reference, &terminal).map_err(|e| (e, true))?;
            Ok(terminal)
        })();
        match saved {
            Ok(terminal) => terminal,
            Err((error, report)) => {
                if report
                    && super::topology_handoff::reportable_failure(&error)
                    && let Some(observed) = &observed
                {
                    write_pending(reference, observed, output, writer)?;
                }
                return Err(error);
            }
        }
    };
    let pending = Journal::decode_delivery_origin(reference, terminal.original.as_bytes())?;
    let report = retained_report(
        reference,
        delivery_plan(&pending)?,
        &terminal.completed,
        &terminal.progress,
    )?;
    let bytes = if output.format == OutputFormat::Json {
        format!("{}\n", serde_json::json!({"delivery":report})).into_bytes()
    } else {
        super::setup::render_text(&report).into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    remove_completed_files(journal, reference)?;
    Ok(report)
}
/// Bounded presentation of this invocation's admitted unfinished delivery.
/// Reporting performs no read or mutation; null means unknown, never absent.
fn write_pending<W: Write>(
    reference: &IntentRef,
    observed: &DeliveryObservation,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let plan = &observed.plan;
    let staged = &observed.staged;
    let namespace = &plan.payload.namespace;
    let context = crate::protocol::output::ContinuationContext {
        state_dir: Some(namespace.state_dir.to_string_lossy().into_owned()),
        host: Some(namespace.host_endpoint.to_string_lossy().into_owned()),
    };
    let prefix = super::hook::cli_prefix(&context);
    let mut retry = vec![
        "env".into(),
        format!("HERDR_PANE_ID={}", observed.caller_target.as_str()),
    ];
    retry.extend(prefix.clone());
    retry.extend(["retry".into(), reference.recovery_ref()]);
    let mut inspect = prefix;
    inspect.push("pending-ops".into());
    let completion = observed.phase == "complete";
    let terminal = observed.phase == "terminal";
    let uncertain = match observed.phase {
        "begin" => Some("begin"),
        "create" if staged.thread.is_none() => Some("thread"),
        "invite" if staged.invitation.is_none() && staged.invitation_attempted => {
            Some("invitation")
        }
        "send" if staged.message.is_none() => Some("message"),
        "complete" => Some("completion"),
        _ => None,
    };
    let last = observed.last_observed.as_ref();
    let mut report = serde_json::json!({
        "phase":observed.phase,"failed":true,
        "outcome":if completion {"completion_uncertain"} else if terminal {"completed_terminal_unconfirmed"} else {"pending"},
        "namespace":namespace,"recovery_ref":reference.recovery_ref(),
        "compound":plan.payload.keys.compound,"recipient":plan.recipient,
        "thread":staged.thread,"invitation":staged.invitation,"message":staged.message,
        "invitation_attempted":staged.invitation_attempted,"uncertain":uncertain,
        "completion":if completion {"uncertain"} else if terminal {"observed"} else {"not_attempted"},
        "last_observed_state":last.map(|v| v.state),"status_is_last_observed":last.is_some(),
        "status_unknown":last.is_none(),"staged_source":"retained_local_progress_and_this_invocation",
        "retry_argv":retry,"inspect_argv":inspect,
        "manual_launch_after_confirming_no_start_argv":null,
        "guidance":"thread/invitation/message combine retained local progress (a client-local hint, not canonical authority) with results returned to this invocation; null means unknown, not proof that the child was not created. last_observed_state is the last canonical status this invocation observed, not current state. Retry the exact original reference; it replays the same keyed children and never duplicates them. completed_terminal_unconfirmed means canonical completion was observed but the local terminal record was not confirmed durable; retry presents the retained result without new effects. Delivery never launches a harness, accepts an invitation or ACKs."
    });
    let bytes = if output.format == OutputFormat::Json {
        format!("{}\n", serde_json::json!({"delivery_pending":report})).into_bytes()
    } else {
        let map = report.as_object_mut().unwrap();
        for (key, value) in map.iter_mut() {
            if key.ends_with("_argv")
                && let Ok(argv) = serde_json::from_value::<Vec<String>>(value.clone())
            {
                *value = crate::protocol::output::format_command_argv(&argv).into();
            }
        }
        super::setup::render_text(&report).into_bytes()
    };
    if bytes.len() > 1024 * 1024 {
        return Err(super::invalid_request("oversized delivery pending report"));
    }
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}
fn remove_completed_files(journal: &Journal, reference: &IntentRef) -> io::Result<()> {
    match fs::remove_file(handoff::progress_path(journal, reference)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    match journal.complete(reference) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    File::open(journal.root())?.sync_all()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomaticProgress {
    thread: Option<crate::protocol::ids::ThreadId>,
    message: Option<CommandResult>,
    possible_start: bool,
    launch: Option<serde_json::Value>,
    report: Option<serde_json::Value>,
    report_digest: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomaticTerminal {
    version: u32,
    original: String,
    completed: HandoffResult,
    progress: AutomaticProgress,
    progress_digest: String,
}

fn automatic_terminal_path(journal: &Journal, reference: &IntentRef) -> std::path::PathBuf {
    journal.root().join(format!(
        "automatic-{:020}-{}.terminal",
        reference.ordinal,
        reference.operation.as_str()
    ))
}

fn automatic_original(
    pending: &PendingIntent,
) -> Result<crate::protocol::handoff::AutomaticOriginal, RunError> {
    use crate::protocol::handoff::AutomaticOriginal;
    let SemanticMutation::Frozen { claim, mutation } = &pending.semantic else {
        return Err(super::invalid_request(
            "automatic handoff needs frozen caller",
        ));
    };
    let original = match mutation.as_ref() {
        SemanticMutation::AutomaticDelivery(plan) => AutomaticOriginal::Delivery(plan.clone()),
        SemanticMutation::AutomaticHandoff(plan) => AutomaticOriginal::Handoff(plan.clone()),
        _ => return Err(super::invalid_request("not an automatic handoff origin")),
    };
    let envelope = automatic_envelope(claim, original.clone())?;
    if pending.header.scope != envelope.scope || pending.header.semantic_digest != envelope.digest {
        return Err(super::invalid_request(
            "automatic original scope or digest mismatch",
        ));
    }
    Ok(original)
}

fn load_automatic_terminal(
    journal: &Journal,
    reference: &IntentRef,
) -> Result<Option<AutomaticTerminal>, RunError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(automatic_terminal_path(journal, reference))
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    const MAX: usize = 2 * crate::cli::journal::MAX_AUTOMATIC_ORIGIN_BYTES + 4 * 1024 * 1024;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX as u64 {
        return Err(super::invalid_request(
            "unsafe or oversized automatic terminal",
        ));
    }
    let mut bytes = Vec::new();
    file.take((MAX + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX {
        return Err(super::invalid_request("oversized automatic terminal"));
    }
    let terminal: AutomaticTerminal = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if serde_json::from_slice::<serde_json::Value>(&bytes).map_err(io::Error::other)?
        != serde_json::to_value(&terminal).map_err(io::Error::other)?
        || terminal.version != 2
        || terminal.progress_digest != retained_digest(&terminal.progress)?
    {
        return Err(super::invalid_request("invalid automatic terminal"));
    }
    let pending = Journal::decode_automatic_origin(reference, terminal.original.as_bytes())?;
    let original = automatic_original(&pending)?;
    automatic_report(
        reference,
        &original,
        &terminal.completed,
        &terminal.progress,
    )?;
    match journal.snapshot_automatic_origin(reference) {
        Ok(bytes) if bytes != terminal.original.as_bytes() => {
            return Err(super::invalid_request(
                "automatic terminal origin contradiction",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if handoff::progress_path(journal, reference).try_exists()? {
        let saved: AutomaticProgress = handoff::load_progress(journal, reference)?;
        if serde_json::to_value(saved).map_err(io::Error::other)?
            != serde_json::to_value(&terminal.progress).map_err(io::Error::other)?
        {
            return Err(super::invalid_request(
                "automatic terminal progress contradiction",
            ));
        }
    }
    Ok(Some(terminal))
}

fn save_automatic_terminal(
    journal: &Journal,
    reference: &IntentRef,
    terminal: &AutomaticTerminal,
) -> Result<(), RunError> {
    let bytes = serde_json::to_vec(terminal).map_err(io::Error::other)?;
    const MAX: usize = 2 * crate::cli::journal::MAX_AUTOMATIC_ORIGIN_BYTES + 4 * 1024 * 1024;
    if bytes.len() > MAX {
        return Err(super::invalid_request("oversized automatic terminal"));
    }
    let temp = journal
        .root()
        .join(format!(".automatic-terminal-{}", uuid::Uuid::new_v4()));
    let save = || -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::hard_link(&temp, automatic_terminal_path(journal, reference))?;
        File::open(journal.root())?.sync_all()
    };
    let result = save();
    let cleanup = fs::remove_file(&temp);
    result?;
    cleanup?;
    File::open(journal.root())?.sync_all()?;
    Ok(())
}

fn automatic_report(
    reference: &IntentRef,
    original: &crate::protocol::handoff::AutomaticOriginal,
    completed: &HandoffResult,
    progress: &AutomaticProgress,
) -> Result<serde_json::Value, RunError> {
    let keys = &original.delivery().payload.keys;
    let saved_report = progress
        .report
        .as_ref()
        .ok_or_else(|| super::invalid_request("automatic report is missing"))?;
    if completed.state != HandoffState::Completed
        || completed.compound != keys.compound
        || progress.thread.is_none()
        || completed.thread != progress.thread
        || !matches!(&progress.message, Some(CommandResult::MessageSent(id)) if !id.as_str().is_empty())
        || matches!(
            original,
            crate::protocol::handoff::AutomaticOriginal::Handoff(_)
        ) && (progress.possible_start
            || progress
                .launch
                .as_ref()
                .is_none_or(|v| v["outcome"] != "started"))
        || matches!(
            original,
            crate::protocol::handoff::AutomaticOriginal::Delivery(_)
        ) && (progress.possible_start || progress.launch.is_some())
        || progress.report_digest.as_deref() != Some(retained_digest(saved_report)?.as_str())
    {
        return Err(super::invalid_request(
            "incomplete automatic handoff report",
        ));
    }
    let report = serde_json::json!({
        "compound":keys.compound,
        "thread":progress.thread,
        "recipient":original.delivery().recipient,
        "message":progress.message,
        "launch":progress.launch,
        "outcome":if matches!(original, crate::protocol::handoff::AutomaticOriginal::Handoff(_)) {"started"} else {"staged"},
        "recovery_ref":reference.recovery_ref(),
    });
    if saved_report != &report {
        return Err(super::invalid_request(
            "automatic report differs from retained original",
        ));
    }
    Ok(report)
}

pub(crate) fn load_automatic_original(
    journal: &Journal,
    reference: &IntentRef,
) -> Result<PendingIntent, RunError> {
    let pending = if let Some(terminal) = load_automatic_terminal(journal, reference)? {
        Journal::decode_automatic_origin(reference, terminal.original.as_bytes())?
    } else {
        let bytes = journal.snapshot_automatic_origin(reference)?;
        Journal::decode_automatic_origin(reference, &bytes)?
    };
    automatic_original(&pending)?;
    Ok(pending)
}

pub(crate) fn resolve_automatic_recovery_ref(
    journal: &Journal,
    value: &str,
) -> Result<IntentRef, RunError> {
    let ordinal: u64 = value
        .strip_prefix("local:")
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| super::invalid_request("invalid automatic recovery reference"))?;
    if value != format!("local:{ordinal}") {
        return Err(super::invalid_request(
            "noncanonical automatic recovery reference",
        ));
    }
    let terminal_prefix = format!("automatic-{ordinal:020}-");
    let intent_prefix = format!("{ordinal:020}-");
    let mut found = None;
    for entry in fs::read_dir(journal.root())? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if let Some(operation) = name
            .strip_prefix(&terminal_prefix)
            .and_then(|s| s.strip_suffix(".terminal"))
            .or_else(|| {
                name.strip_prefix(&intent_prefix)
                    .and_then(|s| s.strip_suffix(".intent"))
            })
        {
            let reference = IntentRef {
                ordinal,
                operation: OperationId::parse(operation.to_owned())
                    .map_err(super::invalid_request)?,
            };
            load_automatic_original(journal, &reference)?;
            if found
                .as_ref()
                .is_some_and(|previous| previous != &reference)
            {
                return Err(super::invalid_request(
                    "ambiguous automatic recovery reference",
                ));
            }
            found = Some(reference);
        }
    }
    found.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "automatic recovery reference not found",
        )
        .into()
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn retry_automatic_to_writer<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    actor: super::actor_route::InvocationActor,
    namespace: &HandoffNamespace,
    client: &C,
    launcher: Option<&mut dyn handoff::HandoffLauncher>,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<serde_json::Value, RunError> {
    let result = retry_automatic_inner(
        journal, reference, actor, namespace, client, launcher, clock, output, writer,
    );
    if let Err(error) = &result
        && super::topology_handoff::reportable_failure(error)
        && super::retry::preflight_original_actor(
            journal.root(),
            &reference.recovery_ref(),
            actor,
            &output.context,
        )
        .is_ok()
        && let Ok(pending) = load_automatic_original(journal, reference)
        && let Ok(original) = automatic_original(&pending)
        && namespace.instance == original.delivery().payload.namespace.instance
        && namespace.state_dir.as_os_str()
            == original.delivery().payload.namespace.state_dir.as_os_str()
        && namespace.host_endpoint.as_os_str()
            == original
                .delivery()
                .payload
                .namespace
                .host_endpoint
                .as_os_str()
        && let Ok(progress) = handoff::load_progress::<AutomaticProgress>(journal, reference)
    {
        write_automatic_pending(reference, &pending, &original, &progress, output, writer)?;
    }
    result
}

fn write_automatic_pending<W: Write>(
    reference: &IntentRef,
    pending: &PendingIntent,
    original: &crate::protocol::handoff::AutomaticOriginal,
    progress: &AutomaticProgress,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let namespace = &original.delivery().payload.namespace;
    let context = crate::protocol::output::ContinuationContext {
        state_dir: Some(namespace.state_dir.to_string_lossy().into_owned()),
        host: Some(namespace.host_endpoint.to_string_lossy().into_owned()),
    };
    let mut retry = vec![
        "env".into(),
        format!(
            "HERDR_PANE_ID={}",
            pending.semantic.frozen_claim().unwrap().target.as_str()
        ),
    ];
    retry.extend(super::hook::cli_prefix(&context));
    retry.extend(["retry".into(), reference.recovery_ref()]);
    let phase = if progress.thread.is_none() {
        "ensure"
    } else if progress.message.is_none() {
        "send"
    } else if matches!(
        original,
        crate::protocol::handoff::AutomaticOriginal::Handoff(_)
    ) && progress.launch.is_none()
    {
        "launch"
    } else {
        "complete"
    };
    let report = serde_json::json!({
        "phase":phase,
        "outcome":if progress.possible_start {"outcome_unknown"} else {"pending"},
        "namespace":namespace,
        "compound":original.delivery().payload.keys.compound,
        "thread":progress.thread,
        "recipient":original.delivery().recipient,
        "message":progress.message,
        "launch":progress.launch,
        "possible_start":progress.possible_start,
        "recovery_ref":reference.recovery_ref(),
        "retry_argv":retry,
        "guidance":"Retry the exact original reference after inspecting the pair lifecycle. Ensure creates the initial invitation once; a reject or leave needs explicit repair on the same pair. A possible native start must be inspected before any manual launch. No automatic invitation acceptance, ACK or topology cleanup occurs."
    });
    let bytes = if output.format == OutputFormat::Json {
        if matches!(
            original,
            crate::protocol::handoff::AutomaticOriginal::Handoff(_)
        ) {
            format!("{}\n", serde_json::json!({"handoff_pending":report})).into_bytes()
        } else {
            format!("{}\n", serde_json::json!({"delivery_pending":report})).into_bytes()
        }
    } else {
        super::setup::render_text(&report).into_bytes()
    };
    if bytes.len() > 1024 * 1024 {
        return Err(super::invalid_request("oversized automatic pending report"));
    }
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn retry_automatic_inner<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    actor: super::actor_route::InvocationActor,
    namespace: &HandoffNamespace,
    client: &C,
    launcher: Option<&mut dyn handoff::HandoffLauncher>,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<serde_json::Value, RunError> {
    super::retry::preflight_original_actor(
        journal.root(),
        &reference.recovery_ref(),
        actor,
        &output.context,
    )?;
    if resolve_automatic_recovery_ref(journal, &reference.recovery_ref())? != *reference {
        return Err(super::invalid_request(
            "automatic original reference mismatch",
        ));
    }
    let _lock = handoff::lock(journal, reference)?;
    let retained = load_automatic_terminal(journal, reference)?;
    let pending = if let Some(terminal) = &retained {
        Journal::decode_automatic_origin(reference, terminal.original.as_bytes())?
    } else {
        load_automatic_original(journal, reference)?
    };
    let original = automatic_original(&pending)?;
    let claim = pending.semantic.frozen_claim().unwrap().clone();
    let frozen = &original.delivery().payload.namespace;
    namespace.validate().map_err(super::invalid_request)?;
    if namespace.instance != frozen.instance
        || namespace.state_dir.as_os_str() != frozen.state_dir.as_os_str()
        || namespace.host_endpoint.as_os_str() != frozen.host_endpoint.as_os_str()
    {
        return Err(super::invalid_request(
            "automatic current namespace mismatch",
        ));
    }
    let envelope = automatic_envelope(&claim, original.clone())?;
    let call = |request: crate::protocol::handoff::AutomaticDeliveryMutation| {
        client
            .call(
                Command::AutomaticHandoffDelivery(Box::new(request)),
                &super::cooperative_budget(clock),
            )
            .map_err(RunError::from)
    };
    let terminal = if let Some(terminal) = retained {
        let mut status_request = envelope.clone();
        status_request.action = crate::protocol::handoff::AutomaticDeliveryAction::Status;
        let CommandResult::Handoff(status) = call(status_request)? else {
            return Err(super::invalid_request(
                "unexpected automatic historical status",
            ));
        };
        if status != terminal.completed {
            return Err(super::invalid_request(
                "automatic terminal differs from canonical completion",
            ));
        }
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(automatic_terminal_path(journal, reference))?
            .sync_all()?;
        File::open(journal.root())?.sync_all()?;
        terminal
    } else {
        let exact_original = journal.snapshot_automatic_origin(reference)?;
        let mut progress: AutomaticProgress = handoff::load_progress(journal, reference)?;
        if progress.possible_start && progress.launch.is_none() {
            return Err(crate::protocol::results::ApiError::unknown_outcome(
                "automatic handoff possible native start; inspect the exact pane before retry",
            )
            .into());
        }
        let CommandResult::Handoff(mut status) = call(envelope.begin())? else {
            return Err(super::invalid_request("unexpected automatic Begin result"));
        };
        if status.state != HandoffState::Completed {
            let result = call(envelope.ensure())?;
            let thread = crate::protocol::handoff::automatic_selected_thread(&result)
                .map_err(super::invalid_request)?
                .clone();
            if status.thread.as_ref().is_some_and(|saved| saved != &thread)
                || progress
                    .thread
                    .as_ref()
                    .is_some_and(|saved| saved != &thread)
            {
                return Err(super::invalid_request(
                    "automatic Ensure changed selected pair thread",
                ));
            }
            progress.thread = Some(thread.clone());
            handoff::save_progress(journal, reference, &progress)?;
            let sent = call(envelope.send(thread.clone()))?;
            if !matches!(sent, CommandResult::MessageSent(_))
                || progress
                    .message
                    .as_ref()
                    .is_some_and(|saved| saved != &sent)
            {
                return Err(super::invalid_request("automatic Send result mismatch"));
            }
            progress.message = Some(sent);
            handoff::save_progress(journal, reference, &progress)?;
            if let crate::protocol::handoff::AutomaticOriginal::Handoff(plan) = &original
                && progress.launch.is_none()
            {
                let launcher = launcher.ok_or_else(|| {
                    super::invalid_request("automatic launch needs original launcher")
                })?;
                let request = handoff::prepare_automatic_startup(launcher, plan, &claim, &thread)?;
                let launched =
                    launcher.launch(&request, &plan.delivery.recipient, &mut |possible| {
                        if possible {
                            match call(envelope.clone()) {
                                Ok(CommandResult::SeatResolved(seat))
                                    if seat == plan.delivery.recipient => {}
                                Ok(_) => {
                                    return Err(ApiError::invalid_request(
                                        "unexpected automatic start guard",
                                    ));
                                }
                                Err(RunError::Api(e)) => return Err(e),
                                Err(e) => {
                                    return Err(ApiError::new(
                                        ErrorCode::StoreCorrupt,
                                        e.to_string(),
                                    ));
                                }
                            }
                            progress.possible_start = true;
                            handoff::save_progress(journal, reference, &progress)
                        } else {
                            let mut reset = progress.clone();
                            reset.possible_start = false;
                            handoff::save_progress(journal, reference, &reset)
                                .map(|()| progress = reset)
                        }
                        .map_err(|e| ApiError::new(ErrorCode::StoreCorrupt, e.to_string()))
                    })?;
                if launched.report["outcome"] != "started" {
                    return Err(ApiError::unknown_outcome(
                        "automatic native start outcome unknown",
                    )
                    .into());
                }
                progress.possible_start = false;
                progress.launch = Some(launched.report);
                handoff::save_progress(journal, reference, &progress)?;
            }
            let report = serde_json::json!({
                "compound":original.delivery().payload.keys.compound,
                "thread":progress.thread,
                "recipient":original.delivery().recipient,
                "message":progress.message,
                "launch":progress.launch,
                "outcome":if matches!(original, crate::protocol::handoff::AutomaticOriginal::Handoff(_)) {"started"} else {"staged"},
                "recovery_ref":reference.recovery_ref(),
            });
            progress.report_digest = Some(retained_digest(&report)?);
            progress.report = Some(report);
            handoff::save_progress(journal, reference, &progress)?;
            let CommandResult::Handoff(completed) = call(envelope.complete())? else {
                return Err(super::invalid_request(
                    "unexpected automatic Complete result",
                ));
            };
            status = completed;
        }
        automatic_report(reference, &original, &status, &progress)?;
        let terminal = AutomaticTerminal {
            version: 2,
            original: String::from_utf8(exact_original).map_err(io::Error::other)?,
            completed: status,
            progress_digest: retained_digest(&progress)?,
            progress,
        };
        save_automatic_terminal(journal, reference, &terminal)?;
        terminal
    };
    let report = automatic_report(
        reference,
        &original,
        &terminal.completed,
        &terminal.progress,
    )?;
    let bytes = if output.format == OutputFormat::Json {
        let presented = if matches!(
            original,
            crate::protocol::handoff::AutomaticOriginal::Handoff(_)
        ) {
            serde_json::json!({"handoff":report})
        } else {
            serde_json::json!({"delivery":report})
        };
        format!("{presented}\n").into_bytes()
    } else {
        super::setup::render_text(&report).into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    remove_completed_files(journal, reference)?;
    Ok(report)
}
#[cfg(test)]
#[path = "../../tests/cli/handoff_delivery.rs"]
mod tests;
