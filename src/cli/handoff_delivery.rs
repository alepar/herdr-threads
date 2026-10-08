//! Internal existing-peer delivery. Public dispatch remains inert until activation.
//! Seat reads select and conservatively refuse; they cannot authorize a live
//! effect. Activation must supply daemon-selected namespace and deciding current
//! recipient guards in addition to the legacy caller/membership fences.
use super::{
    RunError, handoff,
    journal::{DeliveryPlan, IntentRef, IntentScope, Journal, SemanticMutation},
    topology_handoff::Request,
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
        results::{CommandResult, ContinuityStatus, SeatInspection},
        time::Clock,
    },
};
use serde::{Deserialize, Serialize};
pub struct Preparation<'a, C: LocalClient + ?Sized> {
    pub claim: &'a CallerClaim,
    pub namespace: &'a HandoffNamespace,
    pub topology: &'a crate::host::observation::HostTopology,
    pub client: &'a C,
    pub clock: &'a dyn Clock,
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
    let fail = super::invalid_request;
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
    let pending = journal.load(reference)?;
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
    let current = handoff::keyed_fence(client, clock, &identity, keys.begin.clone(), false)?;
    let mut progress: Progress = handoff::load_progress(journal, reference)?;
    if current.state == HandoffState::Completed {
        return progress
            .report
            .filter(|report| {
                report["thread"] == serde_json::json!(current.thread)
                    && report["recipient"] == serde_json::json!(plan.recipient)
                    && report["outcome"] == "staged"
            })
            .ok_or_else(|| {
                super::invalid_request("completed delivery needs its retained staged-work report")
            });
    }
    let recipient = inspect(client, clock, &plan.recipient)?;
    let mut staged = progress.staged.clone();
    let mut phase = "create";
    let call = |semantic: SemanticMutation, key: &OperationId| -> Result<CommandResult, RunError> {
        Ok(client.call(
            semantic.to_command(key.clone(), Some(claim.clone()))?,
            &super::cooperative_budget(clock),
        )?)
    };
    handoff::stage_work(
        handoff::Staging {
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
    )?;
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
    progress.report = Some(report.clone());
    handoff::save_progress(journal, reference, &progress)?;
    handoff::keyed_fence(client, clock, &identity, keys.complete.clone(), true)?;
    Ok(report)
}
#[cfg(test)]
#[path = "../../tests/cli/handoff_delivery.rs"]
mod tests;
