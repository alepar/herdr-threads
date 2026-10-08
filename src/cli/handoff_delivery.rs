//! Internal existing-peer delivery. Public dispatch remains inert until activation.
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
        results::{CommandResult, ContinuityStatus, SeatInspection},
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
    file.take(4 * 1024 * 1024).read_to_end(&mut bytes)?;
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
    execute_locked(journal, reference, journal.load(reference)?, client, clock)
        .map(|(report, _)| report)
}
fn execute_locked<C: LocalClient + ?Sized>(
    journal: &Journal,
    reference: &IntentRef,
    pending: PendingIntent,
    client: &C,
    clock: &dyn Clock,
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
    let mut progress: Progress = load_delivery_progress(journal, reference)?;
    if current.state == HandoffState::Completed {
        return retained_report(reference, &plan, &current, &progress)
            .map(|report| (report, current));
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
    progress.report_digest = Some(retained_digest(&report)?);
    handoff::save_progress(journal, reference, &progress)?;
    let completed = handoff::keyed_fence(client, clock, &identity, keys.complete.clone(), true)?;
    retained_report(reference, &plan, &completed, &progress)?;
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
    if !file.metadata()?.is_file() || file.metadata()?.len() > 131072 {
        return Err(super::invalid_request(
            "unsafe or oversized delivery terminal record",
        ));
    }
    let mut bytes = Vec::new();
    file.take(131073).read_to_end(&mut bytes)?;
    if bytes.len() > 131072 {
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
fn save_terminal(
    journal: &Journal,
    reference: &IntentRef,
    terminal: &Terminal,
) -> Result<(), RunError> {
    let bytes = serde_json::to_vec(terminal).map_err(io::Error::other)?;
    if bytes.len() > 131072 {
        return Err(super::invalid_request("oversized delivery terminal record"));
    }
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
    if namespace != &plan.payload.namespace {
        return Err(super::invalid_request(
            "delivery current namespace mismatch",
        ));
    }
    let terminal = if let Some(terminal) = retained {
        let claim = pending.semantic.frozen_claim().unwrap().clone();
        let keys = &plan.payload.keys;
        let identity = HandoffIdentity {
            compound: keys.compound.clone(),
            digest: pending.header.semantic_digest.clone(),
            claim,
            thread: plan.payload.channel.thread().cloned(),
            recipient: plan.recipient.clone(),
            create_key: keys.create.clone(),
            invite_key: keys.invite.clone(),
            send_key: keys.send.clone(),
        };
        let canonical = handoff::keyed_fence(client, clock, &identity, keys.begin.clone(), false)?;
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
        let (_, completed) = execute_locked(journal, reference, pending, client, clock)?;
        let progress: Progress = load_delivery_progress(journal, reference)?;
        let progress_digest = retained_digest(&progress)?;
        let terminal = Terminal {
            version: 1,
            original: String::from_utf8(original).map_err(io::Error::other)?,
            completed,
            progress,
            progress_digest,
        };
        save_terminal(journal, reference, &terminal)?;
        terminal
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
#[cfg(test)]
#[path = "../../tests/cli/handoff_delivery.rs"]
mod tests;
