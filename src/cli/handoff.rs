//! Private compound coordinator: frozen payload, exact keyed durable steps,
//! and a persisted possible-start boundary. No implicit recipient acceptance.
use super::{
    RunError,
    journal::{IntentRef, IntentScope, Journal, SemanticMutation},
    launch::LaunchRequest,
};
use crate::{
    ports::LocalClient,
    protocol::{
        authority::CallerClaim,
        commands::Command,
        ids::{OperationId, SeatId, ThreadId},
        output::OutputSpec,
        results::{ApiError, CommandResult, ErrorCode},
        time::Clock,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
};

pub const HANDOFF_HELP: &str =
    "Choose exactly one of --new-thread or --thread ID_OR_NAME, and an explicit --pane.
New threads join the sender; existing threads require a joined sender. --thread-name,
--topic and --goal apply only to new threads. Topic defaults to Handoff to DISPLAY;
goal defaults to topic. --name names the native agent, not the channel.

The one quoted body after -- is durable work. Native options use repeatable
--agent-arg=OPTION; launch -- native arguments is unchanged.
HERDR_THREADS_CODEX_OPTS / HERDR_THREADS_CLAUDE_OPTS prepend optional arguments,
using shell-style quotes and escapes without variable or command expansion. Unset
or empty adds nothing. Handoff freezes these options before preflight; retry uses
the saved arguments even if the environment changes.
Handoff invites and sends before guarded launch. Startup gets fixed inbox
instructions, not a second copy of the body. Launch never accepts or ACKs.

Committed work survives failure. pending-ops lists the compound reference; retry REF
resumes exact keyed steps. A confirmed pre-start refusal can retry after repair.
Possible start, unknown outcome or a crash across submission never auto-launches
again: inspect the reported pane/seat before using the reported manual launch argv.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffRequest {
    pub thread: Option<ThreadId>,
    pub thread_name: Option<String>,
    pub topic: Option<String>,
    pub goal: Option<String>,
    pub body: String,
    pub launch: LaunchRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_input: Option<crate::harness::adapter::StartupInputTemplate>,
    pub request: HandoffRequest,
    pub context: crate::protocol::output::ContinuationContext,
    pub recipient: SeatId,
    pub create_key: OperationId,
    pub invite_key: OperationId,
    pub send_key: OperationId,
}
impl HandoffRequest {
    fn validate(&self) -> io::Result<()> {
        for value in std::iter::once(&self.body)
            .chain(self.topic.iter())
            .chain(self.goal.iter())
        {
            if value.is_empty() || value.len() > 1024 {
                return Err(io::Error::other(
                    "handoff text must contain 1..1024 UTF-8 bytes",
                ));
            }
        }
        if self.thread.is_some()
            && (self.topic.is_some() || self.goal.is_some() || self.thread_name.is_some())
        {
            return Err(io::Error::other("new-thread fields on existing handoff"));
        }
        if let Some(name) = &self.thread_name {
            crate::protocol::commands::validate_thread_name(name).map_err(io::Error::other)?;
        }
        Ok(())
    }
}
impl HandoffPlan {
    pub fn validate(&self) -> io::Result<()> {
        self.request.validate()?;
        if let Some(template) = &self.startup_input {
            template
                .validate(self.request.launch.argv.len())
                .map_err(|error| io::Error::other(error.detail))?;
        }
        if self.request.thread.is_none()
            && (self.request.topic.is_none() || self.request.goal.is_none())
        {
            return Err(io::Error::other("missing new thread topic or goal"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    thread: Option<ThreadId>,
    invitation: Option<CommandResult>,
    message: Option<CommandResult>,
    possible_start: bool,
    launch: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgressPhase {
    UncertainAbsorbing,
    TerminalCompletionEligible,
    HistoricalStartedThin,
    LaunchEligible,
}
fn classify_progress(plan: &HandoffPlan, progress: &Progress) -> ProgressPhase {
    use ProgressPhase::*;
    if plan
        .request
        .thread
        .as_ref()
        .zip(progress.thread.as_ref())
        .is_some_and(|(expected, retained)| expected != retained)
    {
        return UncertainAbsorbing;
    }
    let valid_invite = match (&progress.invitation, &progress.thread) {
        (None, _) => true,
        (Some(CommandResult::Invitation(_)), Some(_)) => true,
        (Some(CommandResult::AlreadyJoined(joined)), Some(thread)) => {
            joined.thread == *thread && joined.seat == plan.recipient
        }
        _ => false,
    };
    let valid_message = match &progress.message {
        None => true,
        Some(CommandResult::MessageSent(_)) => {
            progress.thread.is_some() && progress.invitation.is_some()
        }
        _ => false,
    };
    if !valid_invite || !valid_message {
        return UncertainAbsorbing;
    }
    if let Some(report) = &progress.launch {
        if !report.is_object() || report["outcome"] != "started" || progress.thread.is_none() {
            return UncertainAbsorbing;
        }
        return match (progress.invitation.is_some(), progress.message.is_some()) {
            (true, true) => TerminalCompletionEligible,
            (false, false) => HistoricalStartedThin,
            _ => UncertainAbsorbing,
        };
    }
    if progress.possible_start {
        UncertainAbsorbing
    } else {
        LaunchEligible
    }
}

#[derive(Debug, Clone)]
pub struct StartupPreflight {
    pub recipient: SeatId,
    pub native_argv: Vec<String>,
}

/// Tests and production share this composition contract. Launch must recheck
/// the frozen recipient and invoke the gate immediately before native submit.
pub trait HandoffLauncher {
    fn native_options_env(
        &self,
        request: &LaunchRequest,
    ) -> Result<Option<&'static str>, ApiError> {
        super::launch::native_options_env(crate::harness::registry::builtins(), request.harness)
    }
    fn select_startup_input(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<crate::harness::adapter::StartupInputTemplate, RunError> {
        let registry = crate::harness::registry::builtins();
        let registration =
            crate::harness::launch::launch_registration(registry, request.harness.occupant())?;
        registration
            .launch_policy()
            .expect("checked provider")
            .prepare_startup_input(
                &request.argv,
                &crate::harness::adapter::StartupInputSpec {
                    max_text_bytes: crate::ports::NativeLaunchRequest::MAX_ARG_BYTES,
                },
            )?
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCode::UnsupportedHarness,
                    "selected adapter has no handoff startup input",
                )
                .into()
            })
    }
    /// Synthetic launchers may project their supplied data; production overrides
    /// this with the actual selected owned-input composition stage.
    fn native_input(&mut self, request: &LaunchRequest) -> Result<Vec<String>, RunError> {
        crate::ports::validate_native_argv(&request.argv).map_err(super::invalid_request)?;
        Ok(request.argv.clone())
    }
    fn preflight_startup(
        &mut self,
        original: &LaunchRequest,
        effective: &LaunchRequest,
    ) -> Result<StartupPreflight, RunError> {
        let recipient = self.preflight(original)?;
        let native_argv = self.native_input(effective)?;
        Ok(StartupPreflight {
            recipient,
            native_argv,
        })
    }
    fn preflight_saved(
        &mut self,
        _: &LaunchRequest,
        effective: &LaunchRequest,
        recipient: &SeatId,
    ) -> Result<StartupPreflight, RunError> {
        Ok(StartupPreflight {
            recipient: recipient.clone(),
            native_argv: self.native_input(effective)?,
        })
    }
    fn preflight(&mut self, request: &LaunchRequest) -> Result<SeatId, RunError>;
    /// `gate(true)` precedes submission. `gate(false)` is permitted only
    /// with adapter-proven NotSubmitted evidence, never a guessed error code.
    fn launch(
        &mut self,
        request: &LaunchRequest,
        recipient: &SeatId,
        gate: &mut dyn FnMut(bool) -> Result<(), ApiError>,
    ) -> Result<super::launch::LaunchReport, RunError>;
}
fn progress_path(journal: &Journal, reference: &IntentRef) -> std::path::PathBuf {
    journal
        .root()
        .join(format!("handoff-{}.progress", reference.operation.as_str()))
}
fn save(journal: &Journal, reference: &IntentRef, progress: &Progress) -> io::Result<()> {
    let temp = journal
        .root()
        .join(format!(".handoff-{}", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    serde_json::to_writer(&mut file, progress)?;
    file.sync_all()?;
    fs::rename(temp, progress_path(journal, reference))?;
    File::open(journal.root())?.sync_all()
}
fn load(journal: &Journal, reference: &IntentRef) -> io::Result<Progress> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(progress_path(journal, reference))
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Progress::default()),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    const MAX_PROGRESS_BYTES: u64 = 4 * 1024 * 1024;
    if !metadata.is_file() || metadata.len() > MAX_PROGRESS_BYTES {
        return Err(io::Error::other("unsafe handoff progress"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PROGRESS_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PROGRESS_BYTES {
        return Err(io::Error::other("oversized handoff progress"));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
fn lock(journal: &Journal, reference: &IntentRef) -> io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(
            journal
                .root()
                .join(format!("handoff-{}.lock", reference.operation.as_str())),
        )?;
    file.try_lock()
        .map_err(|_| io::Error::other("handoff already running; retry after it finishes"))?;
    Ok(file)
}

fn membership<C: LocalClient + ?Sized>(
    client: &C,
    thread: &ThreadId,
    claim: &CallerClaim,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    use crate::protocol::{
        commands::{DirectoryMembership, DirectoryQuery},
        pagination::PageRequest,
    };
    // Indexed caller membership directory, with bounded pages and canonical IDs.
    let mut page = PageRequest::default();
    loop {
        let result = client.call(
            Command::Directory(DirectoryQuery {
                recent: false,
                membership: Some(claim.seat.clone()),
                membership_filter: DirectoryMembership::Joined,
                topic_contains: None,
                page: page.clone(),
            }),
            &super::cooperative_budget(clock),
        )?;
        let CommandResult::Directory(found) = result else {
            return Err(super::invalid_request("unexpected membership result"));
        };
        if let Some(row) = found.items.iter().find(|row| &row.thread == thread) {
            return if row.archived {
                Err(ApiError::new(ErrorCode::Archived, "handoff thread is archived").into())
            } else {
                Ok(())
            };
        }
        match found.next_cursor {
            Some(cursor) => page.cursor = Some(cursor),
            None => {
                return Err(ApiError::new(
                    ErrorCode::MembershipRequired,
                    "handoff sender must already be joined",
                )
                .into());
            }
        }
    }
}

struct StartupEnvelope {
    request: LaunchRequest,
    slot_token: String,
    reserve: usize,
}
fn startup_envelope(
    request: &LaunchRequest,
    context: &crate::protocol::output::ContinuationContext,
    instance: &str,
    thread: Option<&ThreadId>,
    template: &crate::harness::adapter::StartupInputTemplate,
) -> Result<StartupEnvelope, RunError> {
    // Bound raw inputs before shell/JSON escaping; this keeps intermediate
    // encodings finite. Complete encoded slot and actual native bounds follow.
    let raw = context
        .state_dir
        .iter()
        .chain(context.host.iter())
        .try_fold(instance.len(), |sum, value| sum.checked_add(value.len()));
    if raw.is_none_or(|bytes| bytes > crate::ports::NativeLaunchRequest::MAX_ARG_BYTES) {
        return Err(super::invalid_request(
            "handoff routing input exceeds bounded encoding",
        ));
    }
    let text = bounded_bootstrap_text(
        thread.map_or("", ThreadId::as_str),
        context,
        instance,
        template.max_arg_bytes,
    )?;
    let reserve = if thread.is_some() { 0 } else { 128 };
    let (argv, slot) = template.apply(&request.argv, &text)?;
    if argv[slot]
        .len()
        .checked_add(reserve)
        .is_none_or(|bytes| bytes > template.max_arg_bytes)
    {
        return Err(super::invalid_request(
            "startup input exceeds complete reserved native argument limit",
        ));
    }
    let slot_token = argv[slot].clone();
    let mut effective = request.clone();
    effective.argv = argv;
    Ok(StartupEnvelope {
        request: effective,
        slot_token,
        reserve,
    })
}
fn validate_whole_input(envelope: &StartupEnvelope, argv: &[String]) -> Result<(), RunError> {
    let mut slots = argv
        .iter()
        .enumerate()
        .filter(|(_, token)| *token == &envelope.slot_token);
    let (slot, _) = slots.next().ok_or_else(|| {
        super::invalid_request("selected preparation did not preserve the opaque startup input")
    })?;
    if slots.next().is_some() {
        return Err(super::invalid_request(
            "selected preparation duplicated or ambiguously preserved startup input",
        ));
    }
    crate::ports::validate_native_argv_with_reserved_bytes(argv, slot, envelope.reserve)
        .map_err(super::invalid_request)
}
fn prepare_saved_startup(
    launcher: &mut dyn HandoffLauncher,
    plan: &HandoffPlan,
    claim: &CallerClaim,
    thread: Option<&ThreadId>,
    template: &crate::harness::adapter::StartupInputTemplate,
) -> Result<StartupEnvelope, RunError> {
    let envelope = startup_envelope(
        &plan.request.launch,
        &plan.context,
        &claim.instance,
        thread,
        template,
    )?;
    let prepared =
        launcher.preflight_saved(&plan.request.launch, &envelope.request, &plan.recipient)?;
    if prepared.recipient != plan.recipient {
        return Err(super::invalid_request("handoff frozen recipient changed"));
    }
    validate_whole_input(&envelope, &prepared.native_argv)?;
    Ok(envelope)
}

#[allow(clippy::too_many_arguments)]
pub fn start<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    mut request: HandoffRequest,
    claim: CallerClaim,
    display: &str,
    client: &C,
    launcher: &mut dyn HandoffLauncher,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    request.validate()?;
    if claim.role != crate::protocol::authority::CallerRole::TopLevel || claim.instance.is_empty() {
        return Err(super::invalid_request(
            "handoff requires top-level caller context",
        ));
    }
    if let Some(thread) = &request.thread {
        membership(client, thread, &claim, clock)?;
    }
    if request.thread.is_none() {
        let topic = request
            .topic
            .get_or_insert_with(|| format!("Handoff to {display}"));
        request.goal.get_or_insert_with(|| topic.clone());
    }
    let template = launcher.select_startup_input(&request.launch)?;
    let envelope = startup_envelope(
        &request.launch,
        &output.context,
        &claim.instance,
        request.thread.as_ref(),
        &template,
    )?;
    let input = launcher.native_input(&envelope.request)?;
    validate_whole_input(&envelope, &input)?;
    let prepared = launcher.preflight_startup(&request.launch, &envelope.request)?;
    validate_whole_input(&envelope, &prepared.native_argv)?;
    let recipient = prepared.recipient;
    let key = || OperationId::new(uuid::Uuid::new_v4().to_string());
    let plan = HandoffPlan {
        startup_input: Some(template),
        request,
        context: output.context.clone(),
        recipient,
        create_key: key(),
        invite_key: key(),
        send_key: key(),
    };
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    let semantic = SemanticMutation::freeze(SemanticMutation::Handoff(Box::new(plan)), claim)?;
    let reference = journal.record(scope.clone(), semantic, clock.utc_now().0)?;
    resume(
        journal, &reference, &scope, client, launcher, clock, output, writer,
    )
}

fn write_report<W: Write>(
    report: &serde_json::Value,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let bytes = if output.format == crate::protocol::output::OutputFormat::Json {
        format!("{}\n", serde_json::json!({"handoff":report})).into_bytes()
    } else {
        super::setup::render_text(report).into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn reconcile_terminal<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    plan: &HandoffPlan,
    claim: &CallerClaim,
    progress: &Progress,
    phase: ProgressPhase,
    identity: &crate::protocol::handoff::HandoffIdentity,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let current = fence(client, clock, identity, false)?;
    if current.thread != progress.thread || current.thread.is_none() {
        return Err(super::invalid_request(
            "handoff canonical and retained thread disagree",
        ));
    }
    if current.state == crate::protocol::handoff::HandoffState::Completed {
        return cleanup_completed(journal, reference, plan, claim, &current, output, writer);
    }
    if phase == ProgressPhase::HistoricalStartedThin {
        let report = report(reference, plan, progress, "launch", true, false, claim);
        write_report(&report, output, writer)?;
        return Err(RunError::Exit(5));
    }
    // Retained full success requests terminal decisions only; no runtime work.
    let report = report(reference, plan, progress, "launch", false, false, claim);
    write_report(&report, output, writer)?;
    let completed = fence(client, clock, identity, true)?;
    if completed.thread != progress.thread {
        return Err(super::invalid_request("handoff completion thread mismatch"));
    }
    journal.complete(reference)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn resume<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    scope: &IntentScope,
    client: &C,
    launcher: &mut dyn HandoffLauncher,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let _lock = lock(journal, reference)?;
    let pending = journal.load(reference)?;
    if &pending.header.scope != scope {
        return Err(super::invalid_request(
            "handoff belongs to a different instance or seat",
        ));
    }
    let SemanticMutation::Frozen { claim, mutation } = pending.semantic else {
        return Err(super::invalid_request("handoff needs frozen caller"));
    };
    let SemanticMutation::Handoff(plan) = *mutation else {
        return Err(super::invalid_request("not a handoff recovery reference"));
    };
    let identity = crate::protocol::handoff::HandoffIdentity {
        compound: reference.operation.clone(),
        digest: pending.header.semantic_digest,
        claim: claim.clone(),
        thread: plan.request.thread.clone(),
        recipient: plan.recipient.clone(),
        create_key: plan.create_key.clone(),
        invite_key: plan.invite_key.clone(),
        send_key: plan.send_key.clone(),
    };
    let mut progress = load(journal, reference)?;
    match classify_progress(&plan, &progress) {
        phase @ (ProgressPhase::TerminalCompletionEligible
        | ProgressPhase::HistoricalStartedThin) => {
            return reconcile_terminal(
                journal, reference, &plan, &claim, &progress, phase, &identity, client, clock,
                output, writer,
            );
        }
        ProgressPhase::UncertainAbsorbing => {
            let mut report = report(reference, &plan, &progress, "launch", true, true, &claim);
            report["manual_launch_after_confirming_no_start_argv"] = serde_json::json!([]);
            report["manual_recovery_error"] =
                serde_json::json!("retained progress is uncertain; inspect before recovery");
            if let Some(template) = &plan.startup_input
                && let Some(thread) = &progress.thread
            {
                // Optional saved-template data projection never selects a new
                // transport, resolves a seat, or changes this absorbing class.
                let projection = startup_envelope(
                    &plan.request.launch,
                    &plan.context,
                    &claim.instance,
                    Some(thread),
                    template,
                )
                .and_then(|envelope| {
                    let native = launcher.native_input(&envelope.request)?;
                    validate_whole_input(&envelope, &native)?;
                    Ok(envelope.request.argv)
                });
                if let Ok(argv) = projection {
                    let mut projected = report_core(
                        reference,
                        &plan,
                        &progress,
                        "launch",
                        true,
                        true,
                        &claim,
                        launcher.native_options_env(&plan.request.launch),
                    );
                    if let Some(manual) =
                        projected["manual_launch_after_confirming_no_start_argv"].as_array_mut()
                        && let Some(separator) = manual.iter().position(|token| token == "--")
                    {
                        manual.truncate(separator + 1);
                        manual.extend(argv.into_iter().map(serde_json::Value::String));
                    }
                    report = projected;
                }
            }
            write_report(&report, output, writer)?;
            return Err(RunError::Exit(5));
        }
        ProgressPhase::LaunchEligible => {}
    }
    let template = match &plan.startup_input {
        Some(template) => template.clone(),
        None => launcher.select_startup_input(&plan.request.launch)?,
    };
    prepare_saved_startup(
        launcher,
        &plan,
        &claim,
        progress.thread.as_ref().or(plan.request.thread.as_ref()),
        &template,
    )?;
    let current = fence(client, clock, &identity, false)?;
    if current.state == crate::protocol::handoff::HandoffState::Completed {
        return Err(super::invalid_request(
            "completed canonical handoff lacks retained successful report",
        ));
    }
    if current
        .thread
        .as_ref()
        .zip(progress.thread.as_ref().or(plan.request.thread.as_ref()))
        .is_some_and(|(canonical, retained)| canonical != retained)
    {
        return Err(super::invalid_request(
            "handoff canonical and retained thread disagree",
        ));
    }
    let mut phase = "create";
    let call = |semantic: SemanticMutation, key: &OperationId| -> Result<CommandResult, RunError> {
        Ok(client.call(
            semantic.to_command(key.clone(), Some(claim.clone()))?,
            &super::cooperative_budget(clock),
        )?)
    };
    let attempt = (|| -> Result<(), RunError> {
        if progress.thread.is_none() {
            progress.thread = match &plan.request.thread {
                Some(thread) => Some(thread.clone()),
                None => match call(
                    SemanticMutation::CreateThread {
                        name: plan.request.thread_name.clone(),
                        topic: plan.request.topic.clone().unwrap(),
                        goal: plan.request.goal.clone().unwrap(),
                    },
                    &plan.create_key,
                )? {
                    CommandResult::ThreadCreated(thread) => Some(thread),
                    _ => return Err(super::invalid_request("unexpected create result")),
                },
            };
            save(journal, reference, &progress)?;
        }
        let thread = progress.thread.clone().unwrap();
        if current
            .thread
            .as_ref()
            .is_some_and(|canonical| canonical != &thread)
        {
            return Err(super::invalid_request(
                "created handoff thread disagrees with canonical fence",
            ));
        }
        let effective = prepare_saved_startup(launcher, &plan, &claim, Some(&thread), &template)?;
        phase = "invite";
        if progress.invitation.is_none() {
            let result = call(
                SemanticMutation::Invite {
                    thread: thread.clone(),
                    seat: plan.recipient.clone(),
                    deadline_millis: None,
                },
                &plan.invite_key,
            )?;
            if !matches!(
                result,
                CommandResult::Invitation(_) | CommandResult::AlreadyJoined(_)
            ) {
                return Err(super::invalid_request("unexpected invite result"));
            }
            progress.invitation = Some(result);
            save(journal, reference, &progress)?;
            if classify_progress(&plan, &progress) == ProgressPhase::UncertainAbsorbing {
                return Err(super::invalid_request(
                    "incompatible retained invitation result",
                ));
            }
        }
        phase = "send";
        if progress.message.is_none() {
            let result = call(
                SemanticMutation::SendMessage {
                    thread: thread.clone(),
                    body: plan.request.body.clone(),
                    invited_recipients: vec![plan.recipient.clone()],
                    deadline_millis: None,
                    relays_user: false,
                    user_intent: None,
                },
                &plan.send_key,
            )?;
            if !matches!(result, CommandResult::MessageSent(_)) {
                return Err(super::invalid_request("unexpected send result"));
            }
            progress.message = Some(result);
            save(journal, reference, &progress)?;
        }
        phase = "launch";
        let launched = launcher.launch(&effective.request, &plan.recipient, &mut |possible| {
            if possible {
                progress.possible_start = true;
                save(journal, reference, &progress)
            } else {
                let mut reset = progress.clone();
                reset.possible_start = false;
                save(journal, reference, &reset).map(|()| progress = reset)
            }
            .map_err(|error| ApiError::new(ErrorCode::StoreCorrupt, error.to_string()))
        });
        match launched {
            Ok(report) => {
                progress.launch = Some(report.report);
                save(journal, reference, &progress)?;
            }
            Err(error) => return Err(error),
        }
        Ok(())
    })();
    let retained_phase = classify_progress(&plan, &progress);
    let unknown = retained_phase == ProgressPhase::UncertainAbsorbing;
    let mut report = if retained_phase == ProgressPhase::TerminalCompletionEligible {
        report(
            reference,
            &plan,
            &progress,
            phase,
            attempt.is_err(),
            unknown,
            &claim,
        )
    } else {
        report_core(
            reference,
            &plan,
            &progress,
            phase,
            attempt.is_err(),
            unknown,
            &claim,
            launcher.native_options_env(&plan.request.launch),
        )
    };
    if retained_phase != ProgressPhase::TerminalCompletionEligible {
        let recovery = progress
            .thread
            .as_ref()
            .ok_or_else(|| super::invalid_request("no canonical thread for runnable recovery"))
            .and_then(|thread| {
                let envelope = startup_envelope(
                    &plan.request.launch,
                    &plan.context,
                    &claim.instance,
                    Some(thread),
                    &template,
                )?;
                let native = launcher.native_input(&envelope.request)?;
                validate_whole_input(&envelope, &native)?;
                Ok(envelope.request.argv)
            });
        match recovery {
            Ok(argv) => {
                if let Some(manual) =
                    report["manual_launch_after_confirming_no_start_argv"].as_array_mut()
                    && let Some(separator) = manual.iter().position(|token| token == "--")
                {
                    manual.truncate(separator + 1);
                    manual.extend(argv.into_iter().map(serde_json::Value::String));
                }
            }
            Err(error) => {
                report["manual_launch_after_confirming_no_start_argv"] = serde_json::json!([]);
                report["manual_recovery_error"] =
                    serde_json::json!(error.to_string().chars().take(512).collect::<String>());
            }
        }
    }
    write_report(&report, output, writer)?;
    if unknown {
        return Err(RunError::Exit(5));
    }
    attempt?;
    if retained_phase != ProgressPhase::TerminalCompletionEligible {
        return Err(super::invalid_request(
            "handoff lacks complete retained successful progress",
        ));
    }
    let completed = fence(client, clock, &identity, true)?;
    if completed.thread != progress.thread {
        return Err(super::invalid_request("handoff completion thread mismatch"));
    }
    journal.complete(reference)?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_completed_retry<C: LocalClient + ?Sized, W: Write>(
    parsed: &super::commands::ParsedCli,
    journal: &Journal,
    instance: &str,
    caller_pane: Option<&str>,
    context: &crate::protocol::output::ContinuationContext,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<bool, RunError> {
    let super::commands::CliAction::Retry(recovery) = &parsed.action else {
        return Ok(false);
    };
    let reference = journal.resolve_recovery_ref(recovery.as_str())?;
    let pending = journal.load(&reference)?;
    if !is_handoff(&pending.semantic) {
        return Ok(false);
    }
    let _lock = lock(journal, &reference)?;
    let SemanticMutation::Frozen { claim, mutation } = pending.semantic else {
        return Err(super::invalid_request("handoff needs frozen caller"));
    };
    let SemanticMutation::Handoff(plan) = *mutation else {
        return Err(super::invalid_request("invalid compound nesting"));
    };
    let scope = IntentScope::Cooperative {
        instance: instance.into(),
        seat: claim.seat.clone(),
    };
    let frozen_routing = super::hook::CommandRouting::from_context(&claim.instance, &plan.context);
    let selected_routing = super::hook::CommandRouting::from_context(instance, context);
    if pending.header.scope != scope
        || claim.instance != instance
        || frozen_routing.is_none()
        || frozen_routing != selected_routing
    {
        return Err(super::invalid_request(
            "handoff requires its exact canonical state directory and host endpoint",
        ));
    }
    // Name/tab/workspace selectors need the ordinary topology resolver. The
    // historical fast path cannot infer their target and must not reject live retry.
    if parsed
        .cooperative_selector
        .as_ref()
        .is_some_and(|selector| selector.direct_id().is_none())
    {
        return Ok(false);
    }
    let selected = if let Some(selection) = &parsed.cooperative {
        let harness = selection.harness.occupant();
        selection.seat == claim.seat
            && selection.target == claim.target
            && selection.role == crate::harness::context::Role::TopLevel
            && harness == claim.harness
            && parsed
                .cooperative_selector
                .as_ref()
                .is_none_or(|selector| selector.direct_id().as_ref() == Some(&claim.target))
    } else {
        caller_pane == Some(claim.target.as_str()) && parsed.cooperative_selector.is_none()
    };
    if !selected {
        return Err(super::invalid_request(
            "completed handoff retry requires its exact frozen caller selection",
        ));
    }
    let identity = crate::protocol::handoff::HandoffIdentity {
        compound: reference.operation.clone(),
        digest: pending.header.semantic_digest,
        claim: claim.clone(),
        thread: plan.request.thread.clone(),
        recipient: plan.recipient.clone(),
        create_key: plan.create_key.clone(),
        invite_key: plan.invite_key.clone(),
        send_key: plan.send_key.clone(),
    };
    let progress = load(journal, &reference)?;
    match classify_progress(&plan, &progress) {
        ProgressPhase::LaunchEligible => Ok(false),
        ProgressPhase::UncertainAbsorbing => Err(super::invalid_request(
            "retained handoff progress is uncertain; inspect without replay",
        )),
        phase => {
            reconcile_terminal(
                journal,
                &reference,
                &plan,
                &claim,
                &progress,
                phase,
                &identity,
                client,
                clock,
                &parsed.output,
                writer,
            )?;
            Ok(true)
        }
    }
}

fn fence<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    identity: &crate::protocol::handoff::HandoffIdentity,
    complete: bool,
) -> Result<crate::protocol::handoff::HandoffResult, RunError> {
    let phase = if complete { "complete" } else { "begin" };
    let mutation = crate::protocol::handoff::HandoffMutation {
        identity: identity.clone(),
        operation: OperationId::new(format!("handoff:{phase}:{}", identity.compound.as_str())),
    };
    let command = if complete {
        Command::CompleteHandoff(mutation)
    } else {
        Command::BeginHandoff(mutation)
    };
    match client.call(command, &super::cooperative_budget(clock))? {
        CommandResult::Handoff(result)
            if result.compound == identity.compound
                && (!complete
                    || result.state == crate::protocol::handoff::HandoffState::Completed) =>
        {
            Ok(result)
        }
        _ => Err(super::invalid_request("unexpected handoff fence result")),
    }
}
fn cleanup_completed<W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    plan: &HandoffPlan,
    claim: &CallerClaim,
    current: &crate::protocol::handoff::HandoffResult,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let progress = load(journal, reference)?;
    if progress.thread != current.thread
        || progress.thread.is_none()
        || progress
            .launch
            .as_ref()
            .is_none_or(|v| v["outcome"] != "started")
    {
        return Err(super::invalid_request(
            "completed handoff requires its retained successful report",
        ));
    }
    let report = report(reference, plan, &progress, "launch", false, false, claim);
    let bytes = if output.format == crate::protocol::output::OutputFormat::Json {
        format!("{}\n", serde_json::json!({"handoff":report})).into_bytes()
    } else {
        super::setup::render_text(&report).into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    journal.complete(reference)?;
    Ok(())
}
fn bootstrap(
    thread: &ThreadId,
    context: &crate::protocol::output::ContinuationContext,
    instance: &str,
) -> String {
    bootstrap_text(thread.as_str(), context, instance)
}
struct BoundedBootstrap {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedBootstrap {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|len| len > self.limit)
        {
            return Err(io::Error::other(
                "handoff encoded bootstrap exceeds bounded native input",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn render_bootstrap<W: Write>(
    thread: &str,
    context: &crate::protocol::output::ContinuationContext,
    instance: &str,
    writer: &mut W,
) -> io::Result<()> {
    writer.write_all("Expected handoff command routing (JSON data): ".as_bytes())?;
    serde_json::to_writer(
        &mut *writer,
        &super::hook::CommandRouting::from_context(instance, context),
    )
    .map_err(io::Error::other)?;
    writer.write_all(" Prefer a startup hook command group only when its instance UUID, canonical state directory and canonical host endpoint exactly match every expected routing field above. Missing (null), different or ambiguous routing cannot supersede this handoff's target. Open your durable inbox using that matching group. Otherwise use the exact fallback: `".as_bytes())?;
    let mut inbox = super::hook::cli_prefix(context);
    inbox.push("inbox".into());
    for (index, token) in inbox.into_iter().enumerate() {
        if index > 0 {
            writer.write_all(b" ")?;
        }
        let encoded = crate::protocol::output::format_command_argv(&[token]);
        writer.write_all(encoded.as_bytes())?;
    }
    write!(
        writer,
        "`. The task for thread {} is stored in inbox; follow its printed next: commands for complete bodies. Do not reread it with read/body. When waiting for replies, finish your turn and let hooks notify you of new mail; do not poll or run follow. Launch does not accept invitations or ACK messages. Accept invitations separately; default text inbox ACKs fully displayed messages.",
        thread
    )
}
fn bootstrap_text(
    thread: &str,
    context: &crate::protocol::output::ContinuationContext,
    instance: &str,
) -> String {
    let mut bytes = Vec::new();
    render_bootstrap(thread, context, instance, &mut bytes).expect("in-memory historical output");
    String::from_utf8(bytes).expect("UTF-8 bootstrap")
}
fn bounded_bootstrap_text(
    thread: &str,
    context: &crate::protocol::output::ContinuationContext,
    instance: &str,
    limit: usize,
) -> Result<String, RunError> {
    let mut writer = BoundedBootstrap {
        bytes: Vec::new(),
        limit,
    };
    render_bootstrap(thread, context, instance, &mut writer)
        .map_err(|error| super::invalid_request(&error.to_string()))?;
    Ok(String::from_utf8(writer.bytes).expect("UTF-8 bootstrap"))
}

fn report(
    reference: &IntentRef,
    plan: &HandoffPlan,
    progress: &Progress,
    phase: &str,
    failed: bool,
    unknown: bool,
    claim: &CallerClaim,
) -> serde_json::Value {
    report_core(
        reference,
        plan,
        progress,
        phase,
        failed,
        unknown,
        claim,
        Ok(None),
    )
}
#[cfg(test)]
fn report_with_registry(
    reference: &IntentRef,
    plan: &HandoffPlan,
    progress: &Progress,
    phase: &str,
    failed: bool,
    unknown: bool,
    context: (&CallerClaim, &crate::harness::registry::Registry),
) -> serde_json::Value {
    let (claim, registry) = context;
    report_core(
        reference,
        plan,
        progress,
        phase,
        failed,
        unknown,
        claim,
        super::launch::native_options_env(registry, plan.request.launch.harness),
    )
}
#[allow(clippy::too_many_arguments)]
fn report_core(
    reference: &IntentRef,
    plan: &HandoffPlan,
    progress: &Progress,
    phase: &str,
    failed: bool,
    unknown: bool,
    claim: &CallerClaim,
    declaration: Result<Option<&'static str>, ApiError>,
) -> serde_json::Value {
    let prefix = super::hook::cli_prefix(&plan.context);
    let mut retry = vec![
        "env".into(),
        format!("HERDR_PANE_ID={}", claim.target.as_str()),
    ];
    retry.extend(prefix.clone());
    retry.extend(["retry".into(), reference.recovery_ref()]);
    let mut inspect = prefix.clone();
    inspect.extend([
        "seat".into(),
        "inspect".into(),
        plan.recipient.as_str().to_owned(),
    ]);
    // This command already carries the frozen configured prefix. Suppress
    // option re-entry for this launch only, including changed/malformed current
    // settings; retain every saved native argument without deduplication.
    let mut manual = vec![
        "env".into(),
        "HERDR_THREADS_CODEX_OPTS=".into(),
        "HERDR_THREADS_CLAUDE_OPTS=".into(),
    ];
    if let Ok(Some(key)) = declaration.as_ref()
        && !matches!(
            *key,
            "HERDR_THREADS_CODEX_OPTS" | "HERDR_THREADS_CLAUDE_OPTS"
        )
    {
        manual.push(format!("{key}="));
    }
    manual.extend(prefix);
    manual.extend([
        "launch".into(),
        "--pane".into(),
        plan.request.launch.target.as_str().to_owned(),
        "--kind".into(),
        plan.request.launch.harness.as_str().into(),
    ]);
    if let Some(name) = &plan.request.launch.name {
        manual.extend(["--name".into(), name.clone()]);
    }
    if let Some(binary) = &plan.request.launch.harness_binary {
        manual.extend(["--harness-binary".into(), binary.clone()]);
    }
    manual.push("--".into());
    if let Some(thread) = &progress.thread {
        let text = bootstrap(thread, &plan.context, &claim.instance);
        if let Some(template) = &plan.startup_input
            && let Ok((argv, _)) = template.apply(&plan.request.launch.argv, &text)
        {
            manual.extend(argv);
        } else {
            // Historical caller arrays are display only; do not reselect a transport.
            manual.extend(plan.request.launch.argv.clone());
            manual.push(text);
        }
    } else {
        manual.clear();
    }
    // Unsafe declarations cannot be serialized into runnable recovery text.
    let recovery_error = declaration.err().map(|error| error.detail);
    if recovery_error.is_some() {
        manual.clear();
    }
    serde_json::json!({"phase":phase,"failed":failed,"outcome":if unknown {"outcome_unknown"} else if failed {"pending"} else {"started"},"thread":progress.thread,"seat":plan.recipient,"pane":plan.request.launch.target,"invitation":progress.invitation,"message":progress.message,"recovery_ref":reference.recovery_ref(),"retry_argv":retry,"inspect_argv":inspect,"manual_launch_after_confirming_no_start_argv":manual,"launch":progress.launch,"manual_recovery_error":recovery_error,"manual_launch_is_historical_display":progress.launch.as_ref().is_some_and(|report| report["outcome"] == "started")})
}

pub(crate) fn is_handoff(semantic: &SemanticMutation) -> bool {
    match semantic {
        SemanticMutation::Frozen { mutation, .. } => is_handoff(mutation),
        SemanticMutation::Handoff(_) => true,
        _ => false,
    }
}
pub(super) struct NativeLauncher<'a> {
    pub(super) parts: super::launch::LaunchParts<'a>,
    pub(super) registry: &'a crate::harness::registry::Registry,
}
impl HandoffLauncher for NativeLauncher<'_> {
    fn native_options_env(
        &self,
        request: &LaunchRequest,
    ) -> Result<Option<&'static str>, ApiError> {
        super::launch::native_options_env(self.registry, request.harness)
    }
    fn select_startup_input(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<crate::harness::adapter::StartupInputTemplate, RunError> {
        let registration =
            crate::harness::launch::launch_registration(self.registry, request.harness.occupant())?;
        registration
            .launch_policy()
            .expect("checked provider")
            .prepare_startup_input(
                &request.argv,
                &crate::harness::adapter::StartupInputSpec {
                    max_text_bytes: crate::ports::NativeLaunchRequest::MAX_ARG_BYTES,
                },
            )?
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCode::UnsupportedHarness,
                    "selected adapter has no handoff startup input",
                )
                .into()
            })
    }
    fn native_input(&mut self, request: &LaunchRequest) -> Result<Vec<String>, RunError> {
        Ok(
            super::launch::prepare_native_input_with_registry(self.registry, request, &self.parts)?
                .argv,
        )
    }
    fn preflight_startup(
        &mut self,
        _: &LaunchRequest,
        effective: &LaunchRequest,
    ) -> Result<StartupPreflight, RunError> {
        let result = super::launch::execute_guarded_with_registry(
            self.registry,
            effective,
            &self.parts,
            true,
            &mut |_| unreachable!(),
        )?;
        Ok(StartupPreflight {
            recipient: serde_json::from_value(result.report["seat"].clone())
                .map_err(io::Error::other)?,
            native_argv: serde_json::from_value(result.report["argv"].clone())
                .map_err(io::Error::other)?,
        })
    }
    fn preflight_saved(
        &mut self,
        original: &LaunchRequest,
        effective: &LaunchRequest,
        _: &SeatId,
    ) -> Result<StartupPreflight, RunError> {
        self.preflight_startup(original, effective)
    }
    fn preflight(&mut self, request: &LaunchRequest) -> Result<SeatId, RunError> {
        let result = super::launch::execute_guarded_with_registry(
            self.registry,
            request,
            &self.parts,
            true,
            &mut |_| unreachable!(),
        )?;
        serde_json::from_value(result.report["seat"].clone())
            .map_err(|e| io::Error::other(e).into())
    }
    fn launch(
        &mut self,
        request: &LaunchRequest,
        recipient: &SeatId,
        gate: &mut dyn FnMut(bool) -> Result<(), ApiError>,
    ) -> Result<super::launch::LaunchReport, RunError> {
        super::launch::execute_guarded_with_registry(
            self.registry,
            request,
            &self.parts,
            false,
            &mut |boundary| match boundary {
                super::launch::LaunchBoundary::BeforeSubmit(native) => {
                    if &native.seat != recipient {
                        return Err(ApiError::new(
                            ErrorCode::TargetUnsafe,
                            "handoff pane now resolves to a different seat; frozen recipient retained",
                        ));
                    }
                    gate(true)
                }
                super::launch::LaunchBoundary::RefusedBeforeStart => gate(false),
            },
        )
    }
}
// A retained non-launch phase must remain readable even when current launch
// configuration is unavailable. If progress changes to eligible concurrently,
// refusal still precedes Begin and every effect.
struct RetainedOnlyLauncher;
impl HandoffLauncher for RetainedOnlyLauncher {
    fn native_input(&mut self, _: &LaunchRequest) -> Result<Vec<String>, RunError> {
        Err(super::invalid_request(
            "retained retry has no current launch inputs",
        ))
    }
    fn preflight(&mut self, _: &LaunchRequest) -> Result<SeatId, RunError> {
        Err(super::invalid_request(
            "retained retry has no current launch inputs",
        ))
    }
    fn launch(
        &mut self,
        _: &LaunchRequest,
        _: &SeatId,
        _: &mut dyn FnMut(bool) -> Result<(), ApiError>,
    ) -> Result<super::launch::LaunchReport, RunError> {
        Err(super::invalid_request(
            "retained retry has no current launch inputs",
        ))
    }
}
fn retained_retry_phase(journal: &Journal, reference: &IntentRef) -> io::Result<ProgressPhase> {
    let pending = journal.load(reference)?;
    let SemanticMutation::Frozen { mutation, .. } = pending.semantic else {
        return Err(io::Error::other("handoff needs frozen caller"));
    };
    let SemanticMutation::Handoff(plan) = *mutation else {
        return Err(io::Error::other("not a handoff recovery reference"));
    };
    Ok(classify_progress(&plan, &load(journal, reference)?))
}
/// Called only after the shared caller mapping/context checks in run_selected.
pub(crate) fn run<W: Write>(
    parsed: super::commands::ParsedCli,
    claim: CallerClaim,
    journal: &Journal,
    paths: &crate::daemon::paths::InstancePaths,
    writer: &mut W,
) -> Result<(), RunError> {
    use std::sync::Arc;
    // Fresh argv was resolved once in top-level dispatch before caller/connection work.
    let clock: Arc<dyn Clock> = Arc::new(super::SystemClock::new());
    let (instance, _, client) = super::connect(paths, &clock)?;
    if let super::commands::CliAction::Retry(recovery) = &parsed.action {
        let reference = journal.resolve_recovery_ref(recovery.as_str())?;
        if retained_retry_phase(journal, &reference)? != ProgressPhase::LaunchEligible {
            let scope = IntentScope::Cooperative {
                instance: claim.instance.clone(),
                seat: claim.seat.clone(),
            };
            return resume(
                journal,
                &reference,
                &scope,
                &client,
                &mut RetainedOnlyLauncher,
                clock.as_ref(),
                &parsed.output,
                writer,
            );
        }
    }
    let mut env = super::setup::SetupEnv::from_process(&parsed.output)?;
    let (runtime, _) =
        super::instance::resolve_context(&super::instance::InstanceInputs::from_process(
            parsed
                .output
                .context
                .state_dir
                .as_ref()
                .map(std::path::PathBuf::from),
            parsed
                .output
                .context
                .host
                .as_ref()
                .map(std::path::PathBuf::from),
        ))?;
    env.state_dir = Some(runtime.state_dir.clone());
    env.host_endpoint = Some(runtime.host_endpoint.clone());
    let mut output = parsed.output.clone();
    output.context.state_dir = Some(runtime.state_dir.display().to_string());
    output.context.host = Some(runtime.host_endpoint.display().to_string());
    let host = crate::host::native::NativeCli::new(runtime.host_endpoint, Arc::clone(&clock));
    let seats = super::launch::DaemonSeatResolver::new(&client, journal, instance, clock.as_ref());
    let shell = super::launch::SystemShellProbe::from_process();
    let mut launcher = NativeLauncher {
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
    match parsed.action {
        super::commands::CliAction::Handoff(mut request) => {
            let labels = host
                .seat_labels(&super::cooperative_budget(clock.as_ref()))
                .unwrap_or_default();
            let display = labels
                .iter()
                .find(|row| row.target == request.launch.target)
                .map(|row| {
                    super::irc::relative_pane_nick(
                        row,
                        labels.iter().find(|row| row.target == claim.target),
                    )
                })
                .unwrap_or_else(|| request.launch.target.as_str().to_owned());
            request.launch.pane_label = labels
                .iter()
                .find(|row| row.target == request.launch.target)
                .and_then(|row| row.pane_label.clone());
            start(
                journal,
                request,
                claim,
                &display,
                &client,
                &mut launcher,
                clock.as_ref(),
                &output,
                writer,
            )
        }
        super::commands::CliAction::Retry(recovery) => {
            let reference = journal.resolve_recovery_ref(recovery.as_str())?;
            let scope = IntentScope::Cooperative {
                instance: claim.instance,
                seat: claim.seat,
            };
            resume(
                journal,
                &reference,
                &scope,
                &client,
                &mut launcher,
                clock.as_ref(),
                &output,
                writer,
            )
        }
        _ => unreachable!("handoff route"),
    }
}

#[cfg(test)]
#[path = "../../tests/cli/handoff.rs"]
mod tests;
