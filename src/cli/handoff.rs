//! Private compound coordinator: frozen payload, exact keyed durable steps,
//! and a persisted possible-start boundary. No implicit recipient acceptance.
use super::{
    RunError,
    journal::{IntentRef, IntentScope, Journal, SemanticMutation},
    launch::LaunchRequest,
};
use crate::store::connection::DurableSync;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<crate::protocol::handoff::HandoffPolicy>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automatic: bool,
    pub thread: Option<ThreadId>,
    pub thread_name: Option<String>,
    pub topic: Option<String>,
    pub goal: Option<String>,
    /// The sender's one-line purpose for the launch prompt (`--summary`, else `--goal`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub body: String,
    pub launch: LaunchRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_input: Option<crate::harness::adapter::StartupInputTemplate>,
    /// Bootstrap plan V2 only: the frozen placement of the frozen V1 prompt text. Distinct from
    /// `startup_input` (the pane path's envelope with today's text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_input: Option<crate::harness::adapter::StartupInputTemplate>,
    /// Template-free (frozen) path only: the bootstrap's frozen summary prompt; absent, V1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_prompt: Option<crate::protocol::handoff::SummaryPrompt>,
    pub request: HandoffRequest,
    pub context: crate::protocol::output::ContinuationContext,
    pub recipient: SeatId,
    pub create_key: OperationId,
    pub invite_key: OperationId,
    pub send_key: OperationId,
}
impl HandoffRequest {
    fn validate(&self) -> io::Result<()> {
        if let Some(summary) = &self.summary {
            summary_prompt::validate(summary).map_err(io::Error::other)?;
        }
        if let Some(policy) = &self.policy {
            policy.validate().map_err(io::Error::other)?;
        }
        if self.body.is_empty() || self.body.len() > super::input::MAX_BODY_BYTES {
            return Err(io::Error::other(
                "handoff body must contain 1..65536 UTF-8 bytes",
            ));
        }
        for value in self.topic.iter().chain(self.goal.iter()) {
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
        if self.request.automatic {
            return Err(io::Error::other(
                "automatic handoff requires its additive V2 plan",
            ));
        }
        self.request.validate()?;
        if let Some(template) = &self.startup_input {
            template
                .validate(self.request.launch.argv.len())
                .map_err(|error| io::Error::other(error.detail))?;
        }
        if let Some(template) = &self.bootstrap_input {
            if self.startup_input.is_some() {
                return Err(io::Error::other(
                    "handoff plan carries both startup and bootstrap input",
                ));
            }
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
    pub policy: Option<&'a crate::protocol::handoff::HandoffPolicy>,
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
            HandoffChannel::Automatic => {
                return Err(super::unsupported("automatic handoff producer unavailable"));
            }
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
                    confirmation: plan.policy.map(|p| p.invitation_confirmation),
                    thread: thread.clone(),
                    seat: plan.recipient.clone(),
                    deadline_millis: plan.policy.and_then(|p| p.invite_deadline_millis),
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
        // The addressed instructions always require the recipient's ACK: priority 0.
        let priority = super::send_priority::resolve(super::send_priority::SendPriorityFlags {
            explicit_ack: true,
            deadline: plan.policy.is_some_and(|p| p.deadline_millis.is_some()),
            ..Default::default()
        })?;
        debug_assert_eq!(priority, crate::protocol::priority::DeliveryPriority::p0());
        // p0 in the spelling a handoff journaled before the priority field used: the handoff plan
        // freezes no wire shape, so a retry of such a handoff must replay its first attempt byte
        // for byte (the daemon derives ordinary -> p0).
        let result = call(
            SemanticMutation::SendMessage {
                delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
                priority: None,
                thread,
                body: plan.body.into(),
                direct_recipient: None,
                invited_recipients: vec![plan.recipient.clone()],
                deadline_millis: plan.policy.and_then(|p| p.deadline_millis),
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
    // One buffer, one write: `to_writer` on the bare file issues a write(2) per
    // JSON token (hundreds for a maximal progress record), and under suite load
    // each of those stalled the CLI's exit for seconds (ht-poh.55).
    let bytes = serde_json::to_vec(progress)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(&bytes)?;
    file.durable_sync()?;
    fs::rename(temp, path)?;
    File::open(journal.root())?.durable_sync()
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
/// How long [`lock`] keeps retrying a contended handoff lock before it refuses.
///
/// `flock` belongs to the open file description, and a process spawn
/// (`posix_spawn` or `fork`) on another thread briefly copies every
/// descriptor, close-on-exec ones included, until the child's exec closes
/// them. A lock this process just released can therefore still read as held
/// for the length of a concurrent spawn (ht-b5yq). A genuinely running
/// handoff holds the lock far longer than this bound and is still refused.
const HANDOFF_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

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
    let start = std::time::Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if start.elapsed() < HANDOFF_LOCK_WAIT => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(_) => {
                return Err(io::Error::other(
                    "handoff already running; retry after it finishes",
                ));
            }
        }
    }
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
    summary: Option<&str>,
    thread: Option<&ThreadId>,
    template: &crate::harness::adapter::StartupInputTemplate,
) -> Result<StartupEnvelope, RunError> {
    // Bound raw inputs before shell/JSON escaping; this keeps intermediate
    // encodings finite. Complete encoded slot and actual native bounds follow.
    let raw = context
        .state_dir
        .iter()
        .chain(context.host.iter())
        .try_fold(summary.map_or(0, str::len), |sum, value| {
            sum.checked_add(value.len())
        });
    if raw.is_none_or(|bytes| bytes > crate::ports::NativeLaunchRequest::MAX_ARG_BYTES) {
        return Err(super::invalid_request(
            "handoff routing input exceeds bounded encoding",
        ));
    }
    let text = pane_prompt(summary, thread.map_or("", ThreadId::as_str), context);
    if text.len() > template.max_arg_bytes {
        return Err(super::invalid_request(
            "handoff startup input exceeds bounded native input",
        ));
    }
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
    thread: Option<&ThreadId>,
    template: &crate::harness::adapter::StartupInputTemplate,
) -> Result<StartupEnvelope, RunError> {
    let envelope = startup_envelope(
        &plan.request.launch,
        &plan.context,
        plan.request.summary.as_deref(),
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

pub(crate) fn prepare_automatic_startup(
    launcher: &mut dyn HandoffLauncher,
    plan: &crate::protocol::handoff::AutomaticHandoffPlan,
    thread: &ThreadId,
) -> Result<LaunchRequest, RunError> {
    let template = plan
        .startup_input
        .as_ref()
        .ok_or_else(|| super::invalid_request("automatic handoff lacks frozen startup template"))?;
    let envelope = startup_envelope(
        &plan.launch,
        &plan.context,
        plan.summary.as_deref(),
        Some(thread),
        template,
    )?;
    let prepared =
        launcher.preflight_saved(&plan.launch, &envelope.request, &plan.delivery.recipient)?;
    if prepared.recipient != plan.delivery.recipient {
        return Err(super::invalid_request(
            "automatic handoff frozen recipient changed",
        ));
    }
    validate_whole_input(&envelope, &prepared.native_argv)?;
    Ok(envelope.request)
}

#[allow(clippy::too_many_arguments)]
fn start_automatic<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    mut request: HandoffRequest,
    claim: CallerClaim,
    client: &C,
    launcher: &mut dyn HandoffLauncher,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    use crate::protocol::handoff::{
        AUTOMATIC_PLAN_VERSION, AutomaticDeliveryPlan, AutomaticHandoffPayload,
        AutomaticHandoffPlan, AutomaticOriginal, HandoffNamespace,
    };
    request.validate()?;
    if !request.automatic
        || request.thread.is_some()
        || request.thread_name.is_some()
        || request.topic.is_some()
        || request.goal.is_some()
    {
        return Err(super::invalid_request(
            "invalid automatic launched-pane request",
        ));
    }
    super::handoff_delivery::require_automatic_capability(client, clock)?;
    super::handoff_delivery::require_policy_capabilities(client, clock, request.policy.as_ref())?;
    // An older daemon refuses the plan's `summary` field: the launch then names only the thread.
    if !client.supports_capability(
        crate::protocol::capabilities::HANDOFF_SUMMARY_PROMPT_V1,
        &super::cooperative_budget(clock),
    ) {
        request.summary = None;
    }
    let namespace = HandoffNamespace {
        instance: claim.instance.clone(),
        state_dir: output
            .context
            .state_dir
            .as_ref()
            .ok_or_else(|| super::invalid_request("automatic handoff needs state directory"))?
            .into(),
        host_endpoint: output
            .context
            .host
            .as_ref()
            .ok_or_else(|| super::invalid_request("automatic handoff needs host endpoint"))?
            .into(),
    };
    namespace.validate().map_err(super::invalid_request)?;
    let template = launcher.select_startup_input(&request.launch)?;
    let envelope = startup_envelope(
        &request.launch,
        &output.context,
        request.summary.as_deref(),
        None,
        &template,
    )?;
    validate_whole_input(&envelope, &launcher.native_input(&envelope.request)?)?;
    let payload = AutomaticHandoffPayload::new(
        namespace.clone(),
        OperationId::new(uuid::Uuid::new_v4().to_string()),
        request.body,
    )
    .and_then(|payload| payload.with_policy(request.policy.clone()))
    .map_err(super::invalid_request)?;
    // Seat allocation follows the complete original and continuation admission.
    let prospective = AutomaticOriginal::Handoff(Box::new(AutomaticHandoffPlan {
        version: AUTOMATIC_PLAN_VERSION,
        delivery: AutomaticDeliveryPlan::new(payload.clone(), SeatId::new("\\".repeat(128)))
            .map_err(super::invalid_request)?,
        launch: request.launch.clone(),
        context: output.context.clone(),
        startup_input: Some(template.clone()),
        summary: request.summary.clone(),
    }));
    super::handoff_delivery::validate_automatic_local_admission(
        journal,
        &super::handoff_delivery::automatic_envelope(&claim, prospective)?,
    )?;
    let prepared = launcher.preflight_startup(&request.launch, &envelope.request)?;
    validate_whole_input(&envelope, &prepared.native_argv)?;
    let delivery =
        AutomaticDeliveryPlan::new(payload, prepared.recipient).map_err(super::invalid_request)?;
    let original = AutomaticOriginal::Handoff(Box::new(AutomaticHandoffPlan {
        version: AUTOMATIC_PLAN_VERSION,
        delivery,
        launch: request.launch,
        context: output.context.clone(),
        startup_input: Some(template),
        summary: request.summary,
    }));
    let preparation = super::handoff_delivery::automatic_envelope(&claim, original.clone())?;
    match client.call(
        Command::AutomaticHandoffDelivery(Box::new(preparation)),
        &super::cooperative_budget(clock),
    )? {
        CommandResult::SeatResolved(seat) if seat == original.delivery().recipient => {}
        _ => {
            return Err(super::invalid_request(
                "unexpected automatic handoff preparation result",
            ));
        }
    }
    let reference = super::handoff_delivery::publish_automatic(journal, original, claim, clock)?;
    super::handoff_delivery::retry_automatic_to_writer(
        journal,
        &reference,
        super::actor_route::InvocationActor::Agent,
        &namespace,
        client,
        Some(launcher),
        clock,
        output,
        writer,
    )?;
    Ok(())
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
    if request.automatic {
        return start_automatic(
            journal, request, claim, client, launcher, clock, output, writer,
        );
    }
    request.validate()?;
    super::require_handoff_capabilities(client, clock, request.policy.as_ref())?;
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
        request.summary.as_deref(),
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
        bootstrap_input: None,
        summary_prompt: None,
        request,
        context: output.context.clone(),
        recipient,
        create_key: key(),
        invite_key: key(),
        send_key: key(),
    };
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        host: claim.host.clone(),
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
                    plan.request.summary.as_deref(),
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
                        crate::harness::registry::builtins(),
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
    super::require_handoff_capabilities(client, clock, plan.request.policy.as_ref())?;
    let template = match &plan.startup_input {
        Some(template) => template.clone(),
        None => launcher.select_startup_input(&plan.request.launch)?,
    };
    prepare_saved_startup(
        launcher,
        &plan,
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
            crate::harness::registry::builtins(),
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
                    plan.request.summary.as_deref(),
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
                policy: plan.request.policy.as_ref(),
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
                    prepare_saved_startup(launcher, plan, progress.thread.as_ref(), template)?;
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
            prepare_saved_startup(launcher, plan, Some(&thread), template)?.request
        } else {
            // Frozen V1 Root input. Its retained report is later validated
            // against frozen V1 composition, so refuse before any start when
            // today's composer would launch different native arguments.
            let mut request = plan.request.launch.clone();
            let prompt = launch_prompt(
                plan.summary_prompt.as_ref(),
                thread.as_str(),
                &plan.context,
                &claim.instance,
            );
            match &plan.bootstrap_input {
                Some(template) => request.argv = template.apply(&request.argv, &prompt)?.0,
                None => request.argv.push(prompt),
            }
            let harness = request.harness.into();
            let current =
                crate::harness::launch::compose_native_argv(harness, request.argv.clone(), vec![])?;
            if crate::harness::launch::bootstrap_v1_harness(harness) {
                if current
                    != crate::harness::launch::compose_bootstrap_v1_argv(
                        harness,
                        request.argv.clone(),
                    )?
                {
                    return Err(super::invalid_request(
                        "current launch composition differs from bootstrap plan V1; a new bootstrap plan version is required",
                    ));
                }
            } else if current
                != crate::harness::launch::compose_bootstrap_v2_argv(harness, request.argv.clone())?
            {
                return Err(super::invalid_request(
                    "current launch composition differs from bootstrap plan V2; a new bootstrap plan version is required",
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
        {
            let report = launched?;
            progress.launch = Some(report.report);
            save(progress)?;
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
    let original = super::retry::load_original_for_actor(journal, recovery.as_str())?;
    if matches!(&original.semantic,
        SemanticMutation::Frozen { mutation, .. }
            if matches!(mutation.as_ref(), SemanticMutation::AutomaticHandoff(_))
    ) {
        return Ok(false);
    }
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
        host: claim.host.clone(),
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
/// Today's pane-path prompt. Pane launches are not validated against frozen prompt bytes, so
/// a retained pane plan renders with today's text.
fn pane_prompt(
    summary: Option<&str>,
    thread: &str,
    context: &crate::protocol::output::ContinuationContext,
) -> String {
    #[allow(unused_mut)]
    let mut text = summary_prompt::render(summary, thread, context);
    #[cfg(any(test, feature = "test-support"))]
    if let Some(suffix) = crate::harness::launch::current_drift::prompt_suffix() {
        text.push_str(suffix);
    }
    text
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
        instance: crate::node::NodeUuid,
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
            instance: instance.parse::<crate::node::NodeUuid>().ok()?,
            state_dir: state.to_str()?.to_owned(),
            host_endpoint: endpoint.to_str()?.to_owned(),
        })
    }

    /// The routing object of an already canonical retained namespace, taken
    /// verbatim. Validation never repeats today's filesystem lookup.
    fn retained(instance: &str, context: &ContinuationContext) -> Option<Routing> {
        Some(Routing {
            instance: instance.parse::<crate::node::NodeUuid>().ok()?,
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

    pub(super) fn needs_escape(ch: char) -> bool {
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

    /// The frozen pinned fallback command `herdr-threads [--state-dir S] [--host-endpoint H]
    /// inbox`, shell-quoted as V1 quoted it. Shared by the summary prompt.
    pub(super) fn fallback_inbox(context: &ContinuationContext, out: &mut String) {
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
            shell_token(token, out);
        }
    }

    fn render(thread: &str, routing: Option<&Routing>, context: &ContinuationContext) -> String {
        let mut out = String::from(PREFIX);
        match routing {
            None => out.push_str("null"),
            Some(routing) => {
                out.push_str("{\"instance\":\"");
                out.push_str(&routing.instance.to_string());
                out.push_str("\",\"state_dir\":");
                json_string(&routing.state_dir, &mut out);
                out.push_str(",\"host_endpoint\":");
                json_string(&routing.host_endpoint, &mut out);
                out.push('}');
            }
        }
        out.push_str(INSTRUCTION);
        fallback_inbox(context, &mut out);
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
    /// composition of `caller` plus one retained prompt (V1 text, or the summary
    /// prompt when the plan froze a summary). The frozen grammar still
    /// refuses caller arguments V1 never admitted.
    pub(crate) fn retained_argv_matches(
        harness: crate::protocol::authority::Harness,
        caller: &[String],
        prompt: Option<&crate::protocol::handoff::SummaryPrompt>,
        thread: &str,
        context: &ContinuationContext,
        instance: &str,
        retained: Option<&serde_json::Value>,
    ) -> Result<bool, crate::protocol::results::ApiError> {
        for prompt in super::retained_prompts(prompt, thread, context, instance) {
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
/// Launch prompt of the summary-prompt version: a plan that froze a
/// [`crate::protocol::handoff::SummaryPrompt`] launches with this one line instead of the V1
/// how-to text; the full body arrives as the thread message. The summary is sender-written
/// text (`--summary`, else `--goal`) and appears in native argv and launch reports; the body
/// never does. Retained successful launch reports are validated against these exact bytes:
/// once released, never edit them; a different prompt needs a new frozen version.
pub(crate) mod summary_prompt {
    use crate::protocol::output::ContinuationContext;

    /// Largest summary, in UTF-8 bytes.
    pub(crate) const MAX_BYTES: usize = 200;
    const LEAD: &str = "Handoff: ";
    const THREAD: &str = " \u{2014} full task in thread ";
    const NO_SUMMARY: &str = "see thread ";
    const FALLBACK: &str = " (arrives as a herdr-threads message; if not shown, run `";
    const END: &str = "`).";

    /// A sender-written summary: one line of 1..=200 bytes without control characters.
    pub(crate) fn validate(summary: &str) -> Result<(), &'static str> {
        if summary.trim().is_empty()
            || summary.len() > MAX_BYTES
            || summary.chars().any(super::bootstrap_v1::needs_escape)
        {
            return Err(
                "--summary must be one line of 1..200 UTF-8 bytes without control, line \
                 separator or format characters",
            );
        }
        Ok(())
    }

    /// The sender's summary for a launch: `--summary`, else a `--goal` that passes
    /// [`validate`] (one line of at most 200 bytes); a longer goal gives no summary.
    pub(crate) fn select(summary: Option<&str>, goal: Option<&str>) -> Option<String> {
        summary
            .or(goal.filter(|goal| validate(goal).is_ok()))
            .map(str::to_owned)
    }

    pub(crate) fn render(
        summary: Option<&str>,
        thread: &str,
        context: &ContinuationContext,
    ) -> String {
        let mut out = String::from(LEAD);
        match summary {
            Some(summary) => {
                out.push_str(summary);
                out.push_str(THREAD);
            }
            None => out.push_str(NO_SUMMARY),
        }
        out.push_str(thread);
        out.push_str(FALLBACK);
        super::bootstrap_v1::fallback_inbox(context, &mut out);
        out.push_str(END);
        out
    }
}

/// Stderr hint for a launching handoff without a usable summary: no `--summary` and no `--goal`
/// that fits one line of 200 bytes (91 bytes).
pub(crate) const SUMMARY_HINT: &str =
    "hint: add --summary 'one-line purpose' so the launched agent's first prompt states the task";

/// Whether this action launches an agent whose first prompt will name only the thread.
pub(crate) fn launch_lacks_summary(action: &super::commands::CliAction) -> bool {
    match action {
        super::commands::CliAction::Handoff(request) => request.summary.is_none(),
        super::commands::CliAction::TopologyHandoff(request) => {
            !request.existing && request.summary.is_none()
        }
        _ => false,
    }
}

/// The frozen prompt a new topology plan records: the summary prompt, or None (the frozen V1
/// prompt) only when the daemon predates it and would refuse the field.
pub(crate) fn frozen_prompt<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    summary: Option<&str>,
) -> Option<crate::protocol::handoff::SummaryPrompt> {
    client
        .supports_capability(
            crate::protocol::capabilities::HANDOFF_SUMMARY_PROMPT_V1,
            &super::cooperative_budget(clock),
        )
        .then(|| crate::protocol::handoff::SummaryPrompt {
            summary: summary.map(str::to_owned),
        })
}

/// The prompt a launch of a frozen plan carries: the summary prompt when it froze one, else
/// the frozen V1 text (pre-upgrade records and plans made against an older daemon).
pub(crate) fn launch_prompt(
    prompt: Option<&crate::protocol::handoff::SummaryPrompt>,
    thread: &str,
    context: &crate::protocol::output::ContinuationContext,
    instance: &str,
) -> String {
    match prompt {
        Some(prompt) => summary_prompt::render(prompt.summary.as_deref(), thread, context),
        None => bootstrap_v1::prompt(thread, context, instance),
    }
}

/// Every prompt a launch of this frozen plan could have retained: the one summary prompt,
/// or the V1 outcomes for a plan without one.
pub(crate) fn retained_prompts(
    prompt: Option<&crate::protocol::handoff::SummaryPrompt>,
    thread: &str,
    context: &crate::protocol::output::ContinuationContext,
    instance: &str,
) -> Vec<String> {
    match prompt {
        Some(prompt) => vec![summary_prompt::render(
            prompt.summary.as_deref(),
            thread,
            context,
        )],
        None => bootstrap_v1::retained_prompts(thread, context, instance),
    }
}

/// Bootstrap plan V2 retained-launch validation: the adapter's own composition, frozen into the
/// plan. The caller arguments must still compose to the frozen `composed` argv (else the
/// adapter drifted and the plan is refused), and the retained successful report's `argv` must
/// be that composition of the caller arguments plus one generated bootstrap prompt. The
/// prompt text is harness-neutral and shared with V1.
#[allow(clippy::too_many_arguments)]
pub(crate) fn retained_argv_matches_v2(
    harness: crate::protocol::authority::Harness,
    caller: &[String],
    composed: &[String],
    template: &crate::harness::adapter::StartupInputTemplate,
    prompt: Option<&crate::protocol::handoff::SummaryPrompt>,
    thread: &str,
    context: &crate::protocol::output::ContinuationContext,
    instance: &str,
    retained: Option<&serde_json::Value>,
) -> Result<bool, crate::protocol::results::ApiError> {
    if crate::harness::launch::compose_bootstrap_v2_argv(harness, caller.to_vec())? != composed {
        return Err(crate::protocol::results::ApiError::new(
            crate::protocol::results::ErrorCode::Conflict,
            format!(
                "bootstrap plan V2: {} now composes its launch arguments differently from the frozen plan",
                harness.as_str()
            ),
        ));
    }
    for text in retained_prompts(prompt, thread, context, instance) {
        let Ok((argv, _)) = template.apply(caller, &text) else {
            continue;
        };
        let expected = crate::harness::launch::compose_bootstrap_v2_argv(harness, argv)?;
        if retained == Some(&serde_json::json!(expected)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Retained-launch validation for a bootstrap launch of either plan version: V2 when the plan
/// froze the adapter's composition, else the frozen V1 grammar.
pub(crate) fn retained_launch_argv_matches(
    launch: &crate::protocol::handoff::BootstrapLaunch,
    thread: &str,
    context: &crate::protocol::output::ContinuationContext,
    instance: &str,
    retained: Option<&serde_json::Value>,
) -> Result<bool, crate::protocol::results::ApiError> {
    match &launch.composed_argv {
        Some(composed) => retained_argv_matches_v2(
            launch.harness,
            &launch.argv,
            composed,
            launch.startup_input.as_ref().ok_or_else(|| {
                crate::protocol::results::ApiError::new(
                    crate::protocol::results::ErrorCode::Conflict,
                    "bootstrap plan V2 carries no frozen startup input",
                )
            })?,
            launch.summary_prompt.as_ref(),
            thread,
            context,
            instance,
            retained,
        ),
        None => bootstrap_v1::retained_argv_matches(
            launch.harness,
            &launch.argv,
            launch.summary_prompt.as_ref(),
            thread,
            context,
            instance,
            retained,
        ),
    }
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
        crate::harness::registry::builtins(),
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
        registry,
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
    registry: &crate::harness::registry::Registry,
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
    let mut manual = vec!["env".to_owned()];
    let mut keys = super::launch::registered_options_keys(registry, |_| true);
    if let Ok(Some(key)) = declaration.as_ref()
        && !keys.contains(key)
    {
        keys.push(key);
    }
    manual.extend(keys.into_iter().map(|key| format!("{key}=")));
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
            let text = pane_prompt(
                plan.request.summary.as_deref(),
                thread.as_str(),
                &plan.context,
            );
            if let Ok((argv, _)) = template.apply(&plan.request.launch.argv, &text) {
                manual.extend(argv);
            } else {
                manual.extend(plan.request.launch.argv.clone());
                manual.push(text);
            }
        } else {
            // Historical caller arrays are display only; do not reselect a
            // transport. Template-free plans carry the frozen V1 input.
            let prompt = launch_prompt(
                plan.summary_prompt.as_ref(),
                thread.as_str(),
                &plan.context,
                &claim.instance,
            );
            match plan
                .bootstrap_input
                .as_ref()
                .map(|template| template.apply(&plan.request.launch.argv, &prompt))
            {
                Some(Ok((argv, _))) => manual.extend(argv),
                _ => {
                    manual.extend(plan.request.launch.argv.clone());
                    manual.push(prompt);
                }
            }
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
        SemanticMutation::Handoff(_) | SemanticMutation::AutomaticHandoff(_) => true,
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
#[allow(clippy::too_many_arguments)]
fn retry_automatic_live<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    recovery: &str,
    current_claim: &CallerClaim,
    actor: super::actor_route::InvocationActor,
    namespace: &crate::protocol::handoff::HandoffNamespace,
    client: &C,
    launcher: &mut dyn HandoffLauncher,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<bool, RunError> {
    let original = super::retry::load_original_for_actor(journal, recovery)?;
    if !matches!(&original.semantic,
        SemanticMutation::Frozen { mutation, .. }
            if matches!(mutation.as_ref(), SemanticMutation::AutomaticHandoff(_))
    ) {
        return Ok(false);
    }
    if original.header.scope
        != (IntentScope::Cooperative {
            instance: current_claim.instance.clone(),
            host: current_claim.host.clone(),
            seat: current_claim.seat.clone(),
        })
        || original.semantic.frozen_claim() != Some(current_claim)
    {
        return Err(super::invalid_request(
            "live retry differs from original caller",
        ));
    }
    super::handoff_delivery::require_automatic_capability(client, clock)?;
    super::handoff_delivery::retry_automatic_to_writer(
        journal,
        &original.header.reference,
        actor,
        namespace,
        client,
        Some(launcher),
        clock,
        output,
        writer,
    )?;
    Ok(true)
}
/// Called only after the shared caller mapping/context checks in run_selected.
pub(crate) fn run<W: Write>(
    parsed: super::commands::ParsedCli,
    claim: CallerClaim,
    journal: &Journal,
    paths: &crate::daemon::paths::NodePaths,
    writer: &mut W,
) -> Result<(), RunError> {
    use std::sync::Arc;
    // Fresh argv was resolved once in top-level dispatch before caller/connection work.
    let clock: Arc<dyn Clock> = Arc::new(super::SystemClock::new());
    let (instance, _, client) = super::connect(paths, &clock)?;
    if let super::commands::CliAction::Retry(recovery) = &parsed.action {
        let original = super::retry::load_original_for_actor(journal, recovery.as_str())?;
        let automatic = matches!(&original.semantic,
            SemanticMutation::Frozen { mutation, .. }
                if matches!(mutation.as_ref(), SemanticMutation::AutomaticHandoff(_))
        );
        if !automatic
            && retained_retry_phase(journal, &original.header.reference)?
                != ProgressPhase::LaunchEligible
        {
            let reference = original.header.reference;
            let scope = IntentScope::Cooperative {
                instance: claim.instance.clone(),
                host: claim.host.clone(),
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
    let claim_host = runtime.require_host()?.to_path_buf();
    env.host_endpoint = Some(claim_host.clone());
    let mut output = parsed.output.clone();
    output.context.state_dir = Some(runtime.state_dir.display().to_string());
    output.context.host = Some(claim_host.display().to_string());
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: instance.to_string(),
        state_dir: runtime.state_dir.clone(),
        host_endpoint: claim_host.clone(),
    };
    let host = crate::host::native::NativeCli::new(claim_host, Arc::clone(&clock));
    let seats = super::launch::DaemonSeatResolver::new(
        &client,
        journal,
        instance,
        runtime.claim_host()?,
        clock.as_ref(),
    );
    let shell = super::launch::SystemShellProbe::from_process();
    let mut launcher = NativeLauncher {
        registry: crate::harness::registry::builtins(),
        parts: super::launch::LaunchParts {
            env: &env,
            host: &host,
            seats: &seats,
            handoff: &client,
            clock: clock.as_ref(),
            record_dir: Some(&paths.node_dir),
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
            if retry_automatic_live(
                journal,
                recovery.as_str(),
                &claim,
                parsed.actor,
                &namespace,
                &client,
                &mut launcher,
                clock.as_ref(),
                &output,
                writer,
            )? {
                return Ok(());
            }
            let original = super::retry::load_original_for_actor(journal, recovery.as_str())?;
            let reference = original.header.reference;
            let scope = IntentScope::Cooperative {
                instance: claim.instance,
                host: claim.host.clone(),
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

/// Hash the exact generated JSON without retaining a second diagnostic buffer.
pub(crate) fn automatic_summary(report: &serde_json::Value) -> io::Result<serde_json::Value> {
    use sha2::{Digest, Sha256};
    struct HashWriter(Sha256);
    impl Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, report)?;
    let digest = format!("{:x}", writer.0.finalize());
    if report["outcome"] == "started" {
        crate::protocol::handoff::automatic_launch_summary(report, &digest)
            .map_err(io::Error::other)
    } else {
        Ok(
            serde_json::json!({"format":"automatic_launch_noncompletion_v1", "observed_started":false,
            "full_report_digest":digest,"diagnostics":"optional_local_artifact"}),
        )
    }
}
pub(crate) fn automatic_diagnostics_path(
    journal: &Journal,
    reference: &IntentRef,
) -> std::path::PathBuf {
    journal.root().join(format!(
        "automatic-diagnostics-{}.json",
        reference.operation.as_str()
    ))
}
/// Critical started progress MUST be durable before this optional diagnostic write.
/// This file is never read to authorize, recover or complete a handoff.
pub(crate) fn save_automatic_diagnostics(
    journal: &Journal,
    reference: &IntentRef,
    report: &serde_json::Value,
) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let temp = journal
        .root()
        .join(format!(".automatic-diagnostics-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        let mut writer = io::BufWriter::new(file);
        serde_json::to_writer(&mut writer, report)?;
        writer.flush()?;
        writer.get_ref().durable_sync()?;
        fs::hard_link(&temp, automatic_diagnostics_path(journal, reference))?;
        File::open(journal.root())?.durable_sync()
    })();
    let cleanup = fs::remove_file(&temp);
    result?;
    cleanup?;
    File::open(journal.root())?.durable_sync()
}
pub(crate) fn automatic_diagnostics_marker(
    journal: &Journal,
    reference: &IntentRef,
    summary: &serde_json::Value,
) -> serde_json::Value {
    let path = automatic_diagnostics_path(journal, reference);
    let state = match fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_file() => "local_file",
        Err(e) if e.kind() == io::ErrorKind::NotFound => "missing",
        _ => "unavailable",
    };
    serde_json::json!({"path":path,"full_report_digest":summary["full_report_digest"],"state":state,"authority":"local_hint"})
}

/// Serialize supported presentation fields with the actual canonical journal root.
/// The UUID/ordinal below dominate future generated references and are only sizes.
pub(crate) fn validate_automatic_presentation_admission(
    root: &std::path::Path,
    namespace: &crate::protocol::handoff::HandoffNamespace,
    summary: &serde_json::Value,
    selection: &crate::protocol::results::DirectThreadEnsured,
    metadata: serde_json::Value,
) -> Result<(), &'static str> {
    let escaped = "\\".repeat(128);
    let reference = IntentRef {
        ordinal: u64::MAX,
        operation: OperationId::new("ffffffff-ffff-ffff-ffff-ffffffffffff"),
    };
    let path = root.join(format!(
        "automatic-diagnostics-{}.json",
        reference.operation.as_str()
    ));
    let mut retry = super::hook::cli_prefix(&crate::protocol::output::ContinuationContext {
        state_dir: namespace.state_dir.to_str().map(str::to_owned),
        host: namespace.host_endpoint.to_str().map(str::to_owned),
    });
    retry.extend(["retry".into(), reference.recovery_ref()]);
    let report = serde_json::json!({"bootstrap":{
        "failed":false,"outcome":"completion_pending","state":"possible_creation",
        "phase":"complete","recovery_ref":reference.recovery_ref(),"namespace":namespace,
        "thread":escaped,"recipient":escaped,"seat":escaped,"pane":escaped,"message":crate::protocol::results::CommandResult::MessageSent(crate::protocol::ids::MessageId::new(escaped.clone())),
        "message_status":"unavailable","launch":summary,"selection":selection.presentation(),
        "local_diagnostics":{"path":path,"full_report_digest":"f".repeat(64),"state":"unavailable","authority":"local_hint"},
        "retry_argv":retry,"metadata":metadata,"guidance":[AUTOMATIC_PENDING_GUIDANCE,AUTOMATIC_BOOTSTRAP_GUIDANCE],
    }});
    // A joint pair of actual wrappers conservatively covers either presentation.
    let repeated = serde_json::json!({"handoff":report,"delivery":report});
    if serde_json::to_vec(&repeated)
        .map_err(|_| "automatic presentation encoding")?
        .len()
        + 1
        > 1024 * 1024
        || super::setup::render_text(&repeated).len() > 1024 * 1024
    {
        return Err("automatic future presentation exceeds 1 MiB");
    }
    Ok(())
}

pub(crate) const AUTOMATIC_PENDING_GUIDANCE: &str = "Retry the exact original reference after inspecting the pair lifecycle. Ensure auto-joins both seats of a new pair (no invitation); a seat that left or rejected needs explicit repair on the same pair. A possible native start must be inspected before any manual launch. No automatic invitation acceptance, ACK or topology cleanup occurs.";
pub(crate) const AUTOMATIC_BOOTSTRAP_GUIDANCE: &str = "Inspect the exact original namespace and attempt. Retry retains original body and keys; unknown native creation or start never grants another automatic submission. No topology cleanup, invitation acceptance or ACK.";

/// Output projection only; the retained canonical/local argv remains exact.
pub(crate) fn automatic_launch_presentation(report: &serde_json::Value) -> serde_json::Value {
    let mut presented = report.clone();
    if presented["format"] == "automatic_launch_summary_v1"
        && let Some(fields) = presented.as_object_mut()
    {
        fields.remove("argv");
    }
    presented
}
