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
        if self.request.thread.is_none()
            && (self.request.topic.is_none() || self.request.goal.is_none())
        {
            return Err(io::Error::other("missing new thread topic or goal"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Progress {
    thread: Option<ThreadId>,
    invitation: Option<CommandResult>,
    message: Option<CommandResult>,
    possible_start: bool,
    launch: Option<serde_json::Value>,
}

/// Tests and production share this composition contract. Launch must recheck
/// the frozen recipient and invoke the gate immediately before native submit.
pub trait HandoffLauncher {
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
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("unsafe handoff progress"));
    }
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024).read_to_end(&mut bytes)?;
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
    let recipient = launcher.preflight(&request.launch)?;
    if request.thread.is_none() {
        let topic = request
            .topic
            .get_or_insert_with(|| format!("Handoff to {display}"));
        request.goal.get_or_insert_with(|| topic.clone());
    }
    let key = || OperationId::new(uuid::Uuid::new_v4().to_string());
    let plan = HandoffPlan {
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
    {
        let current = fence(client, clock, &identity, false)?;
        if current.state == crate::protocol::handoff::HandoffState::Completed {
            return cleanup_completed(journal, reference, &plan, &claim, &current, output, writer);
        }
    }
    let mut progress = load(journal, reference)?;
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
        if progress.launch.is_some() || progress.possible_start {
            return Ok(());
        }
        let mut request = plan.request.launch.clone();
        request
            .argv
            .push(bootstrap(&thread, &plan.context, &claim.instance));
        let launched = launcher.launch(&request, &plan.recipient, &mut |possible| {
            if possible {
                // A failed write may already have published the fence: retain
                // the conservative in-memory outcome as well.
                progress.possible_start = true;
                save(journal, reference, &progress)
            } else {
                // Only adapter-proven NotSubmitted reaches this branch. Keep
                // the old in-memory fence unless its reset is durably saved.
                let mut reset = progress.clone();
                reset.possible_start = false;
                save(journal, reference, &reset).map(|()| progress = reset)
            }
            .map_err(|e| ApiError::new(ErrorCode::StoreCorrupt, e.to_string()))
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
    let unknown = progress.possible_start
        && progress
            .launch
            .as_ref()
            .is_none_or(|value| value["outcome"] != "started");
    let report = report(
        reference,
        &plan,
        &progress,
        phase,
        attempt.is_err(),
        unknown,
        &claim,
    );
    let bytes = if output.format == crate::protocol::output::OutputFormat::Json {
        format!("{}\n", serde_json::json!({"handoff": report})).into_bytes()
    } else {
        super::setup::render_text(&report).into_bytes()
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    if unknown {
        return Err(RunError::Exit(5));
    }
    attempt?;
    {
        fence(client, clock, &identity, true)?;
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
        let harness = match selection.harness {
            crate::harness::context::Harness::Codex => crate::protocol::authority::Harness::Codex,
            crate::harness::context::Harness::Claude => crate::protocol::authority::Harness::Claude,
            crate::harness::context::Harness::Human => crate::protocol::authority::Harness::Human,
        };
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
    let current = fence(client, clock, &identity, false)?;
    if current.state != crate::protocol::handoff::HandoffState::Completed {
        return Ok(false);
    }
    cleanup_completed(
        journal,
        &reference,
        &plan,
        &claim,
        &current,
        &parsed.output,
        writer,
    )?;
    Ok(true)
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
    let mut inbox = super::hook::cli_prefix(context);
    inbox.push("inbox".into());
    let expected = super::hook::CommandRouting::from_context(instance, context);
    let expected = serde_json::to_string(&expected).unwrap_or_else(|_| "null".into());
    format!(
        "Expected handoff command routing (JSON data): {expected} Prefer a startup hook command group only when its instance UUID, canonical state directory and canonical host endpoint exactly match every expected routing field above. Missing (null), different or ambiguous routing cannot supersede this handoff's target. Open your durable inbox using that matching group. Otherwise use the exact fallback: `{}`. The task for thread {} is stored in inbox; follow its printed next: commands for complete bodies. Do not reread it with read/body. When waiting for replies, finish your turn and let hooks notify you of new mail; do not poll or run follow. Launch does not accept invitations or ACK messages. Accept invitations separately; default text inbox ACKs fully displayed messages.",
        crate::protocol::output::format_command_argv(&inbox),
        thread.as_str()
    )
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
    let mut manual = prefix;
    manual.extend([
        "launch".into(),
        "--pane".into(),
        plan.request.launch.target.as_str().to_owned(),
        "--kind".into(),
        match plan.request.launch.harness {
            crate::harness::context::Harness::Codex => "codex",
            _ => "claude",
        }
        .into(),
    ]);
    if let Some(name) = &plan.request.launch.name {
        manual.extend(["--name".into(), name.clone()]);
    }
    if let Some(binary) = &plan.request.launch.harness_binary {
        manual.extend(["--harness-binary".into(), binary.clone()]);
    }
    manual.push("--".into());
    manual.extend(plan.request.launch.argv.clone());
    if let Some(thread) = &progress.thread {
        manual.push(bootstrap(thread, &plan.context, &claim.instance));
    }
    serde_json::json!({"phase":phase,"failed":failed,"outcome":if unknown {"outcome_unknown"} else if failed {"pending"} else {"started"},"thread":progress.thread,"seat":plan.recipient,"pane":plan.request.launch.target,"invitation":progress.invitation,"message":progress.message,"recovery_ref":reference.recovery_ref(),"retry_argv":retry,"inspect_argv":inspect,"manual_launch_after_confirming_no_start_argv":manual,"launch":progress.launch})
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
}
impl HandoffLauncher for NativeLauncher<'_> {
    fn preflight(&mut self, request: &LaunchRequest) -> Result<SeatId, RunError> {
        let result =
            super::launch::execute_guarded(request, &self.parts, true, &mut |_| unreachable!())?;
        serde_json::from_value(result.report["seat"].clone())
            .map_err(|e| io::Error::other(e).into())
    }
    fn launch(
        &mut self,
        request: &LaunchRequest,
        recipient: &SeatId,
        gate: &mut dyn FnMut(bool) -> Result<(), ApiError>,
    ) -> Result<super::launch::LaunchReport, RunError> {
        super::launch::execute_guarded(
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
/// Called only after the shared caller mapping/context checks in run_selected.
pub(crate) fn run<W: Write>(
    mut parsed: super::commands::ParsedCli,
    claim: CallerClaim,
    journal: &Journal,
    paths: &crate::daemon::paths::InstancePaths,
    writer: &mut W,
) -> Result<(), RunError> {
    use std::sync::Arc;
    // Freeze options before preflight or any durable handoff mutations. Retry
    // launches the stored argv and never reads the current options environment.
    if let super::commands::CliAction::Handoff(request) = &mut parsed.action {
        request.launch = request.launch.clone().with_process_options()?;
    }
    let clock: Arc<dyn Clock> = Arc::new(super::SystemClock::new());
    let (instance, _, client) = super::connect(paths, &clock)?;
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
