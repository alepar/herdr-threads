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

pub const HANDOFF_HELP: &str = "Choose exactly one channel: --new-thread or --thread ID_OR_NAME.
Choose one target mode:
  --pane PANE --kind claude|codex launches in an existing explicit pane;
  --new-tab LABEL --kind claude|codex creates a tab and launches there;
  --existing with exactly one --seat SEAT or --pane PANE delivers durable work
    to an existing canonical seat without launching or restarting its session.
New modes require an original top-level Agent and the guarded daemon capability.
--new-tab conflicts with --existing, --tab, --pane and --seat. --cwd is new-tab
only: an absolute existing directory, defaulting to the invocation cwd, normalized
before publication. --space selects one live workspace by exact ID or unique label;
omission for creation uses the caller's live workspace, never UI focus. New-tab is
intentional creation even if its label already exists; it does not focus the tab.
For --existing, --space and --tab may qualify --pane, but conflict with --seat.
Delivery forbids --kind, --harness-binary, --name and --agent-arg and ignores the
launch-option environment. An unresolved, held, retired or foreign seat refuses.
An unbound resolved seat may receive staged work; this does not establish a
working or available session. Delivery reports staged work, not a launch result.

New threads join the sender; existing threads require a joined sender. --thread-name,
--topic and --goal apply only to new threads. Topic defaults to Handoff to DISPLAY:
legacy launch uses the pane display, new-tab uses its label, delivery uses the
canonical seat ID. Goal defaults to topic. --name names the native agent, not the channel.
The one quoted body after -- is durable work (1..1024 UTF-8 bytes), never native argv.
Native launch options use repeatable --agent-arg=OPTION; launch -- native arguments
is unchanged. HERDR_THREADS_CODEX_OPTS / HERDR_THREADS_CLAUDE_OPTS prepend optional
arguments using shell-style quotes and escapes without variable or command
expansion. Unset or empty adds nothing. Launch handoff freezes the combined options
before preflight; retry uses saved arguments even if the environment changes.
Handoff stages an invitation when needed and a message addressed to the exact
recipient; launch modes then perform guarded launch. Startup gets fixed inbox
instructions, not a second copy of the body. Handoff never accepts or ACKs for
the recipient, nor declares task adoption or task completion.
Native execution still needs the selected harness's approval/configuration.

Committed work survives failure. pending-ops lists the compound reference; retry REF
resumes exact keyed steps without duplicate messages or invitations. A confirmed
pre-start refusal can retry after repair. Unknown creation cannot automatically
create another tab: inspect the reported exact namespace and attempt. It is distinct
from downstream possible start; possible start, unknown launch outcome or a crash
across launch submission never automatically launches again. Inspect the exact
downstream pane/seat before using any reported manual launch argv after confirming no
agent started. Manual launch guidance does not prove tab noncreation.
Completed retry presents the retained historical report and cleans its own local
intent without repeating effects or proving current availability.

Administrative bootstrap recovery uses human immediately after the executable:
  herdr-threads human [GLOBALS] handoff recover REF --attempt N --created-pane EXACT_PANE
  herdr-threads human [GLOBALS] handoff recover REF --attempt N --not-created
  herdr-threads human [GLOBALS] handoff recover REF --attempt N --cancel --reason TEXT
Keep the exact reported reference, positive attempt N, state directory and endpoint;
routing/output globals follow immediate human. Root --human selects output only.
Recovery needs the guarded daemon capability and records a separate local-account
operator assertion, preserving the original agent identity. Created-pane asserts
this exact result belongs to the inspected attempt and needs fresh coherent
structural evidence and ordinary guards; labels are not ownership evidence.
Not-created asserts inspected noncreation and quiescence. Cancellation asserts
quiescence and administrative abandonment, not completion; reason is nonblank and
at most 4096 UTF-8 bytes. A known in-flight invocation refuses conflicting recovery;
snapshots or guessed PIDs do not prove quiescence. Recovery never launches downstream
work. Bootstrap cancellation refuses while an exact legacy child fence or live hint
remains; that child may stay protected indefinitely after pane loss or retirement.
Recovery replay presents its exact recorded decision, never authorizes a newer
attempt. Product cleanup never closes created topology.";

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
pub(crate) struct Progress {
    pub thread: Option<ThreadId>,
    pub invitation: Option<CommandResult>,
    pub message: Option<CommandResult>,
    pub possible_start: bool,
    pub launch: Option<serde_json::Value>,
}

/// Exact keyed durable work, independent of native launch and receipt actions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct StagedWork {
    pub thread: Option<ThreadId>,
    pub invitation: Option<CommandResult>,
    pub message: Option<CommandResult>,
    #[serde(default)]
    pub invitation_attempted: bool,
}
pub(crate) struct Staging<'a> {
    pub channel: &'a crate::protocol::handoff::HandoffChannel,
    pub body: &'a str,
    pub recipient: &'a SeatId,
    pub create_key: &'a OperationId,
    pub invite_key: &'a OperationId,
    pub send_key: &'a OperationId,
    pub skip_joined: bool,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn stage_work<C: LocalClient + ?Sized>(
    plan: Staging<'_>,
    progress: &mut StagedWork,
    phase: &mut &'static str,
    call: &dyn Fn(SemanticMutation, &OperationId) -> Result<CommandResult, RunError>,
    save: &mut dyn FnMut(&StagedWork) -> Result<(), RunError>,
    client: &C,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    use crate::protocol::handoff::HandoffChannel;
    *phase = "create";
    if progress.thread.is_none() {
        progress.thread = Some(match plan.channel {
            HandoffChannel::Existing { thread } => thread.clone(),
            HandoffChannel::New { name, topic, goal } => match call(
                SemanticMutation::CreateThread {
                    name: name.clone(),
                    topic: topic.clone(),
                    goal: goal.clone(),
                },
                plan.create_key,
            )? {
                CommandResult::ThreadCreated(thread) => thread,
                _ => return Err(super::invalid_request("unexpected create result")),
            },
        });
        save(progress)?;
    }
    let thread = progress.thread.clone().unwrap();
    *phase = "invite";
    if progress.invitation.is_none() {
        let result = if plan.skip_joined
            && !progress.invitation_attempted
            && recipient_joined(client, clock, &thread, plan.recipient)?
        {
            CommandResult::AlreadyJoined(crate::protocol::results::AlreadyJoined {
                thread: thread.clone(),
                seat: plan.recipient.clone(),
            })
        } else {
            // Retain the keyed invitation decision before a possibly lost reply.
            if plan.skip_joined {
                progress.invitation_attempted = true;
                save(progress)?;
            }
            call(
                SemanticMutation::Invite {
                    thread: thread.clone(),
                    seat: plan.recipient.clone(),
                    deadline_millis: None,
                },
                plan.invite_key,
            )?
        };
        if !matches!(
            result,
            CommandResult::Invitation(_) | CommandResult::AlreadyJoined(_)
        ) {
            return Err(super::invalid_request("unexpected invite result"));
        }
        progress.invitation = Some(result);
        save(progress)?;
    }
    *phase = "send";
    if progress.message.is_none() {
        let result = call(
            SemanticMutation::SendMessage {
                delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
                thread,
                body: plan.body.into(),
                invited_recipients: vec![plan.recipient.clone()],
                deadline_millis: None,
                relays_user: false,
                user_intent: None,
            },
            plan.send_key,
        )?;
        if !matches!(result, CommandResult::MessageSent(_)) {
            return Err(super::invalid_request("unexpected send result"));
        }
        progress.message = Some(result);
        save(progress)?;
    }
    Ok(())
}
pub(crate) fn recipient_joined<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<bool, RunError> {
    use crate::protocol::{
        commands::ParticipantsQuery, pagination::PageRequest, results::MembershipStatus,
    };
    let mut page = PageRequest::default();
    loop {
        let CommandResult::Participants(found) = client.call(
            Command::Participants(ParticipantsQuery {
                thread: thread.clone(),
                page: page.clone(),
                caller: None,
            }),
            &super::cooperative_budget(clock),
        )?
        else {
            return Err(super::invalid_request("unexpected participants result"));
        };
        if let Some(row) = found.items.iter().find(|row| &row.seat == seat) {
            return Ok(row.joined
                && !row.retired
                && row.effective_state == MembershipStatus::Joined
                && row.requirement.is_none());
        }
        match found.next_cursor {
            Some(cursor) => page.cursor = Some(cursor),
            None => return Ok(false),
        }
    }
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
pub(crate) fn progress_path(journal: &Journal, reference: &IntentRef) -> std::path::PathBuf {
    journal
        .root()
        .join(format!("handoff-{}.progress", reference.operation.as_str()))
}
fn save(journal: &Journal, reference: &IntentRef, progress: &Progress) -> io::Result<()> {
    save_progress(journal, reference, progress)
}
pub(crate) fn save_progress<T: Serialize>(
    journal: &Journal,
    reference: &IntentRef,
    progress: &T,
) -> io::Result<()> {
    save_progress_at(journal, &progress_path(journal, reference), progress)
}
pub(crate) fn save_progress_at<T: Serialize>(
    journal: &Journal,
    path: &std::path::Path,
    progress: &T,
) -> io::Result<()> {
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
    fs::rename(temp, path)?;
    File::open(journal.root())?.sync_all()
}
fn load(journal: &Journal, reference: &IntentRef) -> io::Result<Progress> {
    load_progress(journal, reference)
}
pub(crate) fn load_progress<T: serde::de::DeserializeOwned + Default>(
    journal: &Journal,
    reference: &IntentRef,
) -> io::Result<T> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(progress_path(journal, reference))
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(T::default()),
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
pub(crate) fn lock(journal: &Journal, reference: &IntentRef) -> io::Result<File> {
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

pub(crate) fn membership<C: LocalClient + ?Sized>(
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
    let (phase, attempt) = execute_steps(
        &plan,
        &claim,
        &mut progress,
        &mut |progress| {
            save(journal, reference, progress)?;
            if current
                .thread
                .as_ref()
                .zip(progress.thread.as_ref())
                .is_some_and(|(canonical, retained)| canonical != retained)
            {
                return Err(super::invalid_request(
                    "created handoff thread disagrees with canonical fence",
                ));
            }
            Ok(())
        },
        client,
        launcher,
        clock,
        &mut || Ok(()),
        Some(&template),
    );
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
    complete_with(
        &identity,
        OperationId::new(format!("handoff:complete:{}", identity.compound.as_str())),
        &progress,
        &mut |mutation, _, _| {
            keyed_fence(client, clock, &mutation.identity, mutation.operation, true)
        },
    )?;
    journal.complete(reference)?;
    Ok(())
}
/// Shared legacy launch machinery, with a caller-owned durable progress record.
/// The save callback must persist possible-start before native submission.
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_steps<C: LocalClient + ?Sized>(
    plan: &HandoffPlan,
    claim: &CallerClaim,
    progress: &mut Progress,
    save: &mut dyn FnMut(&Progress) -> Result<(), RunError>,
    client: &C,
    launcher: &mut dyn HandoffLauncher,
    clock: &dyn Clock,
    before_start: &mut dyn FnMut() -> Result<(), ApiError>,
    startup_input: Option<&crate::harness::adapter::StartupInputTemplate>,
) -> (&'static str, Result<(), RunError>) {
    let mut phase = "create";
    let call = |semantic: SemanticMutation, key: &OperationId| -> Result<CommandResult, RunError> {
        Ok(client.call(
            semantic.to_command(key.clone(), Some(claim.clone()))?,
            &super::cooperative_budget(clock),
        )?)
    };
    let attempt = (|| -> Result<(), RunError> {
        let channel = match &plan.request.thread {
            Some(thread) => crate::protocol::handoff::HandoffChannel::Existing {
                thread: thread.clone(),
            },
            None => crate::protocol::handoff::HandoffChannel::New {
                name: plan.request.thread_name.clone(),
                topic: plan.request.topic.clone().unwrap(),
                goal: plan.request.goal.clone().unwrap(),
            },
        };
        let mut staged = StagedWork {
            thread: progress.thread.clone(),
            invitation: progress.invitation.clone(),
            message: progress.message.clone(),
            invitation_attempted: false,
        };
        stage_work(
            Staging {
                channel: &channel,
                body: &plan.request.body,
                recipient: &plan.recipient,
                create_key: &plan.create_key,
                invite_key: &plan.invite_key,
                send_key: &plan.send_key,
                skip_joined: false,
            },
            &mut staged,
            &mut phase,
            &call,
            &mut |staged| {
                progress.thread = staged.thread.clone();
                progress.invitation = staged.invitation.clone();
                progress.message = staged.message.clone();
                save(progress)?;
                if let Some(template) = startup_input {
                    if classify_progress(plan, progress) == ProgressPhase::UncertainAbsorbing {
                        return Err(super::invalid_request(
                            "incompatible retained staging result",
                        ));
                    }
                    prepare_saved_startup(
                        launcher,
                        plan,
                        claim,
                        progress.thread.as_ref(),
                        template,
                    )?;
                }
                Ok(())
            },
            client,
            clock,
        )?;
        let thread = staged.thread.clone().unwrap();
        phase = "launch";
        if progress.launch.is_some() || progress.possible_start {
            return Ok(());
        }
        let request = if let Some(template) = startup_input {
            prepare_saved_startup(launcher, plan, claim, Some(&thread), template)?.request
        } else {
            // Frozen V1 Root input. Its retained report is later validated
            // against frozen V1 composition, so refuse before any start when
            // today's composer would launch different native arguments.
            let mut request = plan.request.launch.clone();
            request.argv.push(bootstrap_v1::prompt(
                thread.as_str(),
                &plan.context,
                &claim.instance,
            ));
            let harness = request.harness.into();
            if crate::harness::launch::compose_native_argv(harness, request.argv.clone(), vec![])?
                != crate::harness::launch::compose_bootstrap_v1_argv(harness, request.argv.clone())?
            {
                return Err(super::invalid_request(
                    "current native composition differs from frozen V1 bootstrap contract",
                ));
            }
            request
        };
        let launched = launcher.launch(&request, &plan.recipient, &mut |possible| {
            if possible {
                // Linked bootstrap revalidates the original canonical caller at
                // the actual launcher boundary; standalone retains its contract.
                before_start()?;
                // A failed write may already have published the fence: retain
                // the conservative in-memory outcome as well.
                progress.possible_start = true;
                save(progress)
            } else {
                // Only adapter-proven NotSubmitted reaches this branch. Keep
                // the old in-memory fence unless its reset is durably saved.
                let mut reset = progress.clone();
                reset.possible_start = false;
                save(&reset).map(|()| *progress = reset)
            }
            .map_err(|e| ApiError::new(ErrorCode::StoreCorrupt, e.to_string()))
        });
        match launched {
            Ok(report) => {
                progress.launch = Some(report.report);
                save(progress)?;
            }
            Err(error) => return Err(error),
        }
        Ok(())
    })();
    (phase, attempt)
}
/// Internal completion callback; it never derives identity from a local ref.
/// Linked callers pass the frozen legacy child key and commit both fences in
/// their callback; standalone handoff retains its historical completion command.
pub(crate) fn complete_with<T>(
    identity: &crate::protocol::handoff::HandoffIdentity,
    operation: OperationId,
    progress: &Progress,
    complete: &mut dyn FnMut(
        crate::protocol::handoff::HandoffMutation,
        &ThreadId,
        &serde_json::Value,
    ) -> Result<T, RunError>,
) -> Result<T, RunError> {
    let thread = progress
        .thread
        .as_ref()
        .ok_or_else(|| super::invalid_request("handoff has no retained thread"))?;
    let launch = progress
        .launch
        .as_ref()
        .filter(|v| v["outcome"] == "started")
        .ok_or_else(|| super::invalid_request("handoff requires its retained successful report"))?;
    complete(
        crate::protocol::handoff::HandoffMutation {
            identity: identity.clone(),
            operation,
        },
        thread,
        launch,
    )
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
    keyed_fence(
        client,
        clock,
        identity,
        OperationId::new(format!("handoff:{phase}:{}", identity.compound.as_str())),
        complete,
    )
}
pub(crate) fn keyed_fence<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    identity: &crate::protocol::handoff::HandoffIdentity,
    operation: OperationId,
    complete: bool,
) -> Result<crate::protocol::handoff::HandoffResult, RunError> {
    let mutation = crate::protocol::handoff::HandoffMutation {
        identity: identity.clone(),
        operation,
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
pub(crate) fn bootstrap(
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
    )?;
    #[cfg(any(test, feature = "test-support"))]
    if let Some(suffix) = crate::harness::launch::current_drift::prompt_suffix() {
        writer.write_all(suffix.as_bytes())?;
    }
    Ok(())
}

/// Frozen version-1 Root bootstrap prompt: the single generated native
/// argument a version-1 bootstrap (`BootstrapPlan.version == 1`, also every
/// plan without a startup-input template) appends to its caller arguments.
/// Retained successful launch reports are validated against these exact
/// bytes, so its routing JSON, quoting, token order and prose are a
/// compatibility record: never edit them to follow [`bootstrap`], the
/// current generic renderer used by startup-input templates.
pub(crate) mod bootstrap_v1 {
    use crate::protocol::output::ContinuationContext;
    use std::path::Path;

    const PREFIX: &str = "Expected handoff command routing (JSON data): ";
    const INSTRUCTION: &str = " Prefer a startup hook command group only when its instance UUID, canonical state directory and canonical host endpoint exactly match every expected routing field above. Missing (null), different or ambiguous routing cannot supersede this handoff's target. Open your durable inbox using that matching group. Otherwise use the exact fallback: `";
    const THREAD: &str = "`. The task for thread ";
    const SUFFIX: &str = " is stored in inbox; follow its printed next: commands for complete bodies. Do not reread it with read/body. When waiting for replies, finish your turn and let hooks notify you of new mail; do not poll or run follow. Launch does not accept invitations or ACK messages. Accept invitations separately; default text inbox ACKs fully displayed messages.";
    const ARGV0: &str = "herdr-threads";

    /// Unicode 16.0.0 General_Category=Cf ranges, as V1 quoted them.
    const FORMAT_RANGES: [(u32, u32); 21] = [
        (0x00AD, 0x00AD),
        (0x0600, 0x0605),
        (0x061C, 0x061C),
        (0x06DD, 0x06DD),
        (0x070F, 0x070F),
        (0x0890, 0x0891),
        (0x08E2, 0x08E2),
        (0x180E, 0x180E),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x2064),
        (0x2066, 0x206F),
        (0xFEFF, 0xFEFF),
        (0xFFF9, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0001, 0xE0001),
        (0xE0020, 0xE007F),
    ];

    /// The V1 expected routing object (`instance`, `state_dir`,
    /// `host_endpoint`, in that order).
    pub(crate) struct Routing {
        instance: uuid::Uuid,
        state_dir: String,
        host_endpoint: String,
    }

    /// The routing outcome the V1 producer captures at launch time: the
    /// canonical state directory and endpoint parent, or none when the UUID,
    /// either path or its canonicalization is unavailable.
    fn produced(instance: &str, context: &ContinuationContext) -> Option<Routing> {
        let state = Path::new(context.state_dir.as_deref()?)
            .canonicalize()
            .ok()?;
        let host = Path::new(context.host.as_deref()?);
        let endpoint = host.parent()?.canonicalize().ok()?.join(host.file_name()?);
        Some(Routing {
            instance: uuid::Uuid::parse_str(instance).ok()?,
            state_dir: state.to_str()?.to_owned(),
            host_endpoint: endpoint.to_str()?.to_owned(),
        })
    }

    /// The routing object of an already canonical retained namespace, taken
    /// verbatim. Validation never repeats today's filesystem lookup.
    fn retained(instance: &str, context: &ContinuationContext) -> Option<Routing> {
        Some(Routing {
            instance: uuid::Uuid::parse_str(instance).ok()?,
            state_dir: context.state_dir.clone()?,
            host_endpoint: context.host.clone()?,
        })
    }

    fn json_string(value: &str, out: &mut String) {
        out.push('"');
        for ch in value.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\u{8}' => out.push_str("\\b"),
                '\u{c}' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
                ch => out.push(ch),
            }
        }
        out.push('"');
    }

    fn needs_escape(ch: char) -> bool {
        let cp = ch as u32;
        ch.is_control()
            || matches!(ch, '\u{2028}' | '\u{2029}')
            || FORMAT_RANGES
                .iter()
                .any(|&(low, high)| (low..=high).contains(&cp))
    }

    fn shell_token(arg: &str, out: &mut String) {
        if arg.chars().any(needs_escape) {
            out.push_str("$'");
            for ch in arg.chars() {
                match ch {
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    '\'' => out.push_str("\\'"),
                    '\\' => out.push_str("\\\\"),
                    ch if needs_escape(ch) => {
                        let mut buffer = [0; 4];
                        for byte in ch.encode_utf8(&mut buffer).bytes() {
                            out.push_str(&format!("\\x{byte:02x}"));
                        }
                    }
                    ch => out.push(ch),
                }
            }
            out.push('\'');
        } else if !arg.is_empty()
            && arg
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&byte))
        {
            out.push_str(arg);
        } else {
            out.push('\'');
            for ch in arg.chars() {
                if ch == '\'' {
                    out.push_str("'\\''");
                } else {
                    out.push(ch);
                }
            }
            out.push('\'');
        }
    }

    fn render(thread: &str, routing: Option<&Routing>, context: &ContinuationContext) -> String {
        let mut out = String::from(PREFIX);
        match routing {
            None => out.push_str("null"),
            Some(routing) => {
                out.push_str("{\"instance\":\"");
                out.push_str(&routing.instance.hyphenated().to_string());
                out.push_str("\",\"state_dir\":");
                json_string(&routing.state_dir, &mut out);
                out.push_str(",\"host_endpoint\":");
                json_string(&routing.host_endpoint, &mut out);
                out.push('}');
            }
        }
        out.push_str(INSTRUCTION);
        let mut fallback = vec![ARGV0];
        if let Some(state) = &context.state_dir {
            fallback.extend(["--state-dir", state.as_str()]);
        }
        if let Some(host) = &context.host {
            fallback.extend(["--host-endpoint", host.as_str()]);
        }
        fallback.push("inbox");
        for (index, token) in fallback.into_iter().enumerate() {
            if index > 0 {
                out.push(' ');
            }
            shell_token(token, &mut out);
        }
        out.push_str(THREAD);
        out.push_str(thread);
        out.push_str(SUFFIX);
        out
    }

    /// Produce the V1 prompt for a launch happening now.
    pub(crate) fn prompt(thread: &str, context: &ContinuationContext, instance: &str) -> String {
        render(thread, produced(instance, context).as_ref(), context)
    }

    /// Every prompt a V1 producer could have retained for this canonical
    /// namespace: the verbatim canonical routing object (only for a valid
    /// UUID) and the documented null-routing outcome.
    pub(crate) fn retained_prompts(
        thread: &str,
        context: &ContinuationContext,
        instance: &str,
    ) -> Vec<String> {
        let mut prompts = Vec::with_capacity(2);
        if let Some(routing) = retained(instance, context) {
            prompts.push(render(thread, Some(&routing), context));
        }
        prompts.push(render(thread, None, context));
        prompts
    }

    /// Whether a retained successful report's `argv` is the exact frozen V1
    /// composition of `caller` plus one V1 prompt. The frozen grammar still
    /// refuses caller arguments V1 never admitted.
    pub(crate) fn retained_argv_matches(
        harness: crate::protocol::authority::Harness,
        caller: &[String],
        thread: &str,
        context: &ContinuationContext,
        instance: &str,
        retained: Option<&serde_json::Value>,
    ) -> Result<bool, crate::protocol::results::ApiError> {
        for prompt in retained_prompts(thread, context, instance) {
            let mut argv = caller.to_vec();
            argv.push(prompt);
            let expected = crate::harness::launch::compose_bootstrap_v1_argv(harness, argv)?;
            if retained == Some(&serde_json::json!(expected)) {
                return Ok(true);
            }
        }
        Ok(false)
    }
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

pub(crate) fn report(
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
        if let Some(template) = &plan.startup_input {
            let text = bootstrap(thread, &plan.context, &claim.instance);
            if let Ok((argv, _)) = template.apply(&plan.request.launch.argv, &text) {
                manual.extend(argv);
            } else {
                manual.extend(plan.request.launch.argv.clone());
                manual.push(text);
            }
        } else {
            // Historical caller arrays are display only; do not reselect a
            // transport. Template-free plans carry the frozen V1 input.
            manual.extend(plan.request.launch.argv.clone());
            manual.push(bootstrap_v1::prompt(
                thread.as_str(),
                &plan.context,
                &claim.instance,
            ));
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
