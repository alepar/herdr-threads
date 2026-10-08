//! Typed CLI client, selected renderer and local journal composition.
pub mod actor_route;
pub mod commands;
pub mod doctor;
pub mod exit;
pub mod follow;
pub mod handoff;
pub mod hook;
pub mod hook_evidence;
pub mod human;
pub mod input;
pub mod installer;
mod installer_skill;
pub mod instance;
pub mod internal;
pub mod irc;
pub mod journal;
pub mod launch;
pub mod lazy_display;
pub mod me;
pub mod output;
pub mod panes;
pub(crate) mod peer_locations;
mod picker;
pub mod retry;
pub mod setup;
pub mod skill;
pub mod summary;
pub mod threads;

use crate::{
    app::SystemClock,
    client::local::LocalSocketClient,
    daemon::{
        control::stop_and_wait,
        lifecycle::ensure_running,
        ownership::{read_descriptor, read_existing_namespace},
        paths::{InstancePaths, RuntimeContext},
    },
    ports::LocalClient,
    protocol::{
        commands::Command,
        output::{OutputFormat, OutputSpec},
        results::{ApiError, CommandResult, StopAccepted},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    view::{ViewReader, render_view},
};
use commands::{CliAction, CooperativeSelection, DaemonAction, MutationSpec};
use journal::{IntentScope, SemanticMutation};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[derive(Debug)]
pub enum RunError {
    Api(ApiError),
    Io(io::Error),
    Output(output::OutputError),
    /// The command already rendered its own report; exit with this status.
    Exit(i32),
    /// Usage text for an invocation without a command: printed verbatim on
    /// stderr (no error prefix or code) with the invalid-arguments status.
    Usage(String),
}
impl From<ApiError> for RunError {
    fn from(value: ApiError) -> Self {
        Self::Api(value)
    }
}
impl From<io::Error> for RunError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<output::OutputError> for RunError {
    fn from(value: output::OutputError) -> Self {
        Self::Output(value)
    }
}

fn unsupported(detail: &str) -> RunError {
    RunError::Api(ApiError::unsupported(detail))
}

/// The one-shot CLI never starts the daemon implicitly for reads; say how to.
pub(crate) fn daemon_unavailable() -> RunError {
    RunError::Api(ApiError::host_unavailable(format!(
        "daemon is not running for this state/host context; {}",
        crate::daemon::remedy::remedy(
            Some(crate::protocol::results::ErrorClass::Unavailable),
            &crate::daemon::remedy::RemedyContext::Exit3,
        )
    )))
}

/// Read the published daemon endpoint. Every CLI path that needs the daemon
/// goes through here, so an absent namespace or descriptor (never started,
/// or removed by `daemon stop`) is the stable exit-3 unavailable error rather
/// than a raw Io NotFound.
fn published_endpoint(
    paths: &InstancePaths,
) -> Result<(uuid::Uuid, crate::daemon::ownership::EndpointDescriptor), RunError> {
    let instance = read_existing_namespace(paths)?.ok_or_else(daemon_unavailable)?;
    match read_descriptor(paths, instance) {
        Ok(descriptor) => Ok((instance, descriptor)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(daemon_unavailable()),
        Err(error) => Err(error.into()),
    }
}

/// Report version skew from the descriptor before any wire decode: a live
/// daemon of another protocol is the VersionSkew error; a descriptor whose
/// owner is gone is just an unavailable daemon. `daemon stop` skips this
/// guard (it is skew-tolerant) and so is not routed through [`connect`].
fn skew_guard(
    paths: &InstancePaths,
    instance: uuid::Uuid,
    descriptor: &crate::daemon::ownership::EndpointDescriptor,
) -> Result<(), RunError> {
    if descriptor.protocol_version == crate::protocol::wire::PROTOCOL_VERSION {
        return Ok(());
    }
    match crate::daemon::lifecycle::live_skew(paths, instance, descriptor)? {
        Some(error) => Err(error.into()),
        None => Err(daemon_unavailable()),
    }
}

/// Connect to the published daemon endpoint (see [`published_endpoint`]).
fn connect(
    paths: &InstancePaths,
    clock: &Arc<dyn Clock>,
) -> Result<
    (
        uuid::Uuid,
        crate::daemon::ownership::EndpointDescriptor,
        LocalSocketClient,
    ),
    RunError,
> {
    let (instance, descriptor) = published_endpoint(paths)?;
    // An older daemon drops a newer request at decode with no reply, so refuse
    // here (live skew) instead of sending it; a stale descriptor is unavailable.
    skew_guard(paths, instance, &descriptor)?;
    let client = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(clock),
        instance,
        Some(descriptor.boot_id),
    );
    Ok((instance, descriptor, client))
}

fn mapping_error(detail: &str) -> RunError {
    RunError::Api(ApiError::target_unresolved(detail))
}

/// The refusal for a mutation whose selected seat is unresolved: one line,
/// bounded by construction (seat and pane IDs are at most 128 bytes each,
/// so the line is under 800 bytes).
pub(crate) fn unresolved_seat_refusal(selection: &CooperativeSelection) -> String {
    format!(
        "seat {seat} is unresolved, so nothing was registered or changed; a lifecycle \
         check-in registers once a coherent host snapshot reconfirms pane {pane}; if the seat \
         stays unresolved, `herdr-threads doctor` lists it: repair with `seat rebind {seat} \
         --pane {pane} --operator`",
        seat = selection.seat.as_str(),
        pane = selection.target.as_str(),
    )
}

pub(crate) fn selected_generation(
    selection: &CooperativeSelection,
    inspection: &crate::protocol::results::SeatInspection,
) -> Result<u64, RunError> {
    use crate::protocol::results::ContinuityStatus;
    if inspection.summary.seat == selection.seat
        && inspection.summary.retired_at.is_none()
        && inspection.retirement.is_none()
        && (inspection.summary.continuity == ContinuityStatus::Unresolved
            || inspection.mapping.state == ContinuityStatus::Unresolved)
    {
        // Never a silent refusal (wave-2 fix2 (b)): one bounded stderr line
        // that says why nothing registered, when it will, and the repair.
        return Err(mapping_error(&unresolved_seat_refusal(selection)));
    }
    if inspection.summary.seat != selection.seat
        || inspection.summary.continuity != ContinuityStatus::Resolved
        || inspection.mapping.state != ContinuityStatus::Resolved
        || inspection.summary.target.as_ref() != Some(&selection.target)
        || inspection.mapping.target.as_ref() != Some(&selection.target)
        || inspection.summary.retired_at.is_some()
        || inspection.retirement.is_some()
        || inspection.hold.is_some()
    {
        return Err(mapping_error(
            "selected seat has no current unheld target mapping",
        ));
    }
    Ok(inspection.summary.generation)
}

struct SocketViewReader(LocalSocketClient);
impl ViewReader for SocketViewReader {
    fn read(
        &mut self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.0.call_with_output(command, output, budget)
    }
}

pub fn run<I, T, W>(argv: I, writer: &mut W) -> Result<(), RunError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    W: Write,
{
    let pane = std::env::var("HERDR_PANE_ID").ok();
    run_in_pane(argv, pane.as_deref(), writer)
}

/// Process entrypoint: like [`run`], recording whether stdout is a terminal so
/// text output defaults to the human form for a person and stays in the
/// machine form for pipes, hooks, and models running the CLI through a tool
/// (including a harness that gives its shell tool a PTY: see
/// [`output::HARNESS_MARKERS`]).
pub fn run_terminal<I, T, W>(
    argv: I,
    writer: &mut W,
    stdout_is_terminal: bool,
) -> Result<(), RunError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    W: Write,
{
    output::set_stdout_is_terminal(stdout_is_terminal);
    output::set_harness_marked(output::harness_marked(|name| std::env::var_os(name)));
    run(argv, writer)
}

/// Environment variables an agent harness sets inside its own process
/// (TRUST-POLICY A4, best effort): `CLAUDECODE` (Claude Code) and
/// `CODEX_SANDBOX` / `CODEX_SANDBOX_NETWORK_DISABLED` (set by codex-rs on the
/// commands it spawns). An explicit allowlist, never a prefix: user
/// configuration such as `CODEX_HOME` or `CODEX_API_KEY` is not evidence.
/// No local `codex` was available to confirm the list with
/// `codex exec ... 'env | grep ^CODEX_'`; it follows codex-rs's documented
/// spawn environment.
const AGENT_ENV_MARKERS: [&str; 3] = [
    "CLAUDECODE",
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
];

/// First agent environment marker present, by name.
pub(crate) fn agent_env_marker<K: AsRef<str>, V>(
    vars: impl IntoIterator<Item = (K, V)>,
) -> Option<String> {
    vars.into_iter()
        .map(|(key, _)| key.as_ref().to_owned())
        .find(|key| AGENT_ENV_MARKERS.contains(&key.as_str()))
}

/// The single client-side agent-evidence rule (TRUST-POLICY A4, best effort;
/// a hint, never authority: the daemon's refusal is the guard). Shared by
/// `me init` and person-pane selection. Returns the evidence text: an agent
/// environment marker, else Herdr reporting a supported harness agent in the
/// pane. A failed Herdr read is not evidence. (The launch guard is the
/// required guard itself and keeps propagating its read error.)
pub(crate) fn agent_evidence<K: AsRef<str>, V>(
    env: impl IntoIterator<Item = (K, V)>,
    read_agent: impl FnOnce() -> Result<Option<crate::ports::PaneAgentObservation>, ApiError>,
) -> Option<String> {
    if let Some(marker) = agent_env_marker(env) {
        return Some(format!("environment variable {marker} is set"));
    }
    let observation = read_agent().ok()??;
    let kind = observation.kind?;
    crate::protocol::authority::is_harness_agent_kind(&kind)
        .then(|| format!("Herdr reports a `{kind}` agent in this pane"))
}

/// Refusal shared by `me init` and person-pane commands (TRUST-POLICY A4):
/// agent evidence where a person's identity was about to act.
pub(crate) fn agent_evidence_refusal(pane: &str, evidence: &str) -> RunError {
    invalid_request(&format!(
        "pane {pane}: `me init` and person-pane commands never act as an agent ({evidence}). \
         Run them in your own shell pane, or override as the local account with \
         `herdr-threads me init --operator` (later commands in this pane then run as you)"
    ))
}

/// Single best-effort read for `me init`; never authority. One
/// `observe_pane_agent` call on a fresh adapter with a 2 s budget and no
/// expected boot or epoch.
pub(crate) fn pane_agent(
    context: &RuntimeContext,
    clock: &Arc<dyn Clock>,
    pane: &crate::protocol::ids::HostTargetId,
) -> Result<Option<crate::ports::PaneAgentObservation>, ApiError> {
    use crate::ports::HostPort;
    let host =
        crate::host::native::NativeCli::new(context.host_endpoint.clone(), Arc::clone(clock));
    host.observe_pane_agent(
        pane,
        &crate::ports::HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(clock.monotonic_now().0.saturating_add(2_000)),
                cancellation: Cancellation::default(),
            },
            expected_boot: None,
            expected_epoch: None,
        },
    )
}

/// Run with an explicit caller pane (Herdr's `HERDR_PANE_ID` for the process
/// that invoked the CLI; never the focused pane). The pane only locates the
/// seat whose service mapping and private lifecycle context already exist.
pub fn run_in_pane<I, T, W>(
    argv: I,
    caller_pane: Option<&str>,
    writer: &mut W,
) -> Result<(), RunError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    W: Write,
{
    let mut parsed = match commands::parse_argv_or_informational(argv) {
        Ok(parsed) => parsed,
        Err(commands::ParseFailure::Informational(text)) => {
            writer.write_all(text.as_bytes())?;
            writer.flush()?;
            return Ok(());
        }
        Err(commands::ParseFailure::Usage(text)) => return Err(RunError::Usage(text)),
        Err(commands::ParseFailure::Invalid(error)) => return Err(error.into()),
    };
    let _presentation = output::PresentationGuard::enter(parsed.presentation, &parsed.output);
    if let CliAction::Skill = &parsed.action {
        writer.write_all(skill::SKILL_MD.as_bytes())?;
        writer.flush()?;
        return Ok(());
    }
    if let CliAction::ContractId { harness } = &parsed.action {
        let name = harness.map(harness_name);
        let json = parsed.output.format == OutputFormat::Json;
        writer.write_all(crate::harness::contract::render_contract_ids(name, json).as_bytes())?;
        writer.flush()?;
        return Ok(());
    }
    if let CliAction::HarnessVersionNormalize { harness, raw } = &parsed.action {
        let json = parsed.output.format == OutputFormat::Json;
        let text = crate::harness::contract::render_normalize(harness_name(*harness), raw, json)
            .map_err(|message| invalid_request(&message))?;
        writer.write_all(text.as_bytes())?;
        writer.flush()?;
        return Ok(());
    }
    if let CliAction::InstallerIntegrations { confirm_missing } = &parsed.action {
        return installer::run(*confirm_missing, &parsed.output, writer);
    }
    if let CliAction::InternalJsonField { path } = &parsed.action {
        let mut input = String::new();
        io::Read::read_to_string(&mut io::stdin().lock(), &mut input)?;
        return internal::json_field(&input, path, writer);
    }
    if let CliAction::Doctor { .. } = &parsed.action {
        return doctor::run(&parsed, writer);
    }
    if let CliAction::Setup(request) = &parsed.action {
        return setup::run(request, &parsed.output, writer);
    }
    if let CliAction::SetupAll(verb, prompt_suggestions) = &parsed.action {
        return setup::run_all(*verb, *prompt_suggestions, &parsed.output, writer);
    }
    let (context, _) = instance::resolve_context(&instance::InstanceInputs::from_process(
        parsed.output.context.state_dir.as_ref().map(PathBuf::from),
        parsed.output.context.host.as_ref().map(PathBuf::from),
    ))
    .map_err(context_error)?;
    if let CliAction::Retry(recovery) = &parsed.action {
        let paths = InstancePaths::resolve_read_only(&context)?;
        retry::preflight_original_actor(
            paths.instance_dir.join("intents"),
            recovery.as_str(),
            parsed.actor,
            &crate::protocol::output::ContinuationContext {
                state_dir: Some(context.state_dir.to_string_lossy().into_owned()),
                host: Some(context.host_endpoint.to_string_lossy().into_owned()),
            },
        )?;
    }
    let paths = InstancePaths::resolve(&context)?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(5_000)),
        cancellation: Cancellation::default(),
    };
    let host =
        crate::host::native::NativeCli::new(context.host_endpoint.clone(), Arc::clone(&clock));
    // Canonical terminal cleanup precedes caller selection,
    // SeatInspect and local selected-generation guards. Exact IDs need no host I/O.
    let historical_retry = matches!(&parsed.action, CliAction::Retry(_));
    let deferred_locator = parsed
        .cooperative_selector
        .as_ref()
        .is_some_and(|selector| selector.direct_id().is_none());
    let try_completed = |parsed: &commands::ParsedCli, writer: &mut W| -> Result<bool, RunError> {
        let (instance, _, client) = connect(&paths, &clock)?;
        let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
        let selected_context = crate::protocol::output::ContinuationContext {
            state_dir: Some(context.state_dir.to_string_lossy().into_owned()),
            host: Some(context.host_endpoint.to_string_lossy().into_owned()),
        };
        handoff::try_completed_retry(
            parsed,
            &journal,
            &instance.to_string(),
            caller_pane,
            &selected_context,
            &client,
            clock.as_ref(),
            writer,
        )
    };
    if historical_retry && try_completed(&parsed, writer)? {
        return Ok(());
    }
    panes::resolve_cli_targets(
        &mut parsed,
        || host.topology(&budget()),
        || {
            caller_pane
                .filter(|pane| !pane.is_empty())
                .map(|pane| host.current_pane(pane, &budget()))
                .transpose()
        },
    )?;
    // A deferred name now denotes an exact canonical target; terminal replay
    // still precedes every live caller/SeatInspect/binding-generation guard.
    if historical_retry && deferred_locator && try_completed(&parsed, writer)? {
        return Ok(());
    }
    let connection = LazyConnection::new(|| {
        connect(&paths, &clock).map(|(instance, _, client)| (instance, client))
    });
    let thread_caller = parsed
        .cooperative
        .as_ref()
        .map(|selection| selection.seat.clone());
    let thread_caller_target = parsed
        .cooperative
        .as_ref()
        .map(|selection| selection.target.clone())
        .or_else(|| {
            caller_pane
                .filter(|pane| !pane.is_empty())
                .map(crate::protocol::ids::HostTargetId::new)
        });
    threads::resolve_cli_threads(&mut parsed, |selector| {
        let (_, client) = connection.get()?;
        let result = client
            .call(
                Command::ResolveThread(crate::protocol::commands::ResolveThreadQuery {
                    selector: selector.to_owned(),
                    caller: thread_caller.clone(),
                    caller_target: thread_caller_target.clone(),
                }),
                &budget(),
            )
            .map_err(|error| threads::selector_error(error, selector))?;
        match result {
            CommandResult::ThreadResolved(thread) => Ok(thread),
            _ => Err(invalid_request("unexpected thread resolution result")),
        }
    })?;
    if let CliAction::MeInit { operator } = &parsed.action {
        let operator = *operator;
        return me::run_me_init(
            parsed,
            operator,
            caller_pane,
            &context,
            &paths,
            &clock,
            writer,
        );
    }
    if let CliAction::Launch(request) = &parsed.action {
        return run_launch(request, &parsed, &context, &paths, &clock, writer);
    }
    if let CliAction::Picker(request) = &parsed.action {
        picker::require_terminal(&parsed)?;
        let (_, client) = connection.get()?;
        let Some(thread) = picker::run(|page, refresh, cancellation| {
            let request_budget = CallBudget {
                deadline: MonoInstant(clock.monotonic_now().0.saturating_add(if refresh {
                    1_000
                } else {
                    5_000
                })),
                cancellation,
            };
            if !client
                .capabilities(&request_budget)
                .supports(crate::protocol::capabilities::PICKER_DIRECTORY_V1)
            {
                return Err(ApiError::invalid_request(
                    "channel picker capability unavailable; ensure the daemon is responding and supports picker.directory_v1 (upgrade if needed), or use read THREAD / follow THREAD with an exact thread ID",
                ));
            }
            let result = client.call(
                Command::PickerDirectory(crate::protocol::commands::PickerDirectoryQuery { page }),
                &request_budget,
            )?;
            match result {
                CommandResult::PickerDirectory(page) => Ok(page),
                _ => Err(ApiError::invalid_request(
                    "unexpected picker directory result",
                )),
            }
        })?
        else {
            return Ok(());
        };
        parsed.action = request.selected(thread);
    }
    if let CliAction::Follow(request) = &parsed.action {
        return follow::run(
            request,
            caller_pane,
            &parsed.output,
            &context,
            &paths,
            &clock,
            writer,
        );
    }
    // A person reading a thread gets the IRC transcript with pane nicks and
    // full bodies; every other consumer keeps the selected machine encoding.
    if let CliAction::Wire(Command::History(query)) = &parsed.action
        && output::human_active()
    {
        let (instance, _, client) = connect(&paths, &clock)?;
        let mut cache =
            follow::NickCache::new(&context, &paths, instance, &clock).with_caller(caller_pane);
        return follow::render_history(&client, query.clone(), &parsed.output, &mut cache, writer);
    }
    // Canonical thread and recipient selectors precede both caller composition
    // and the operator branch's durable intent.
    if matches!(&parsed.action, CliAction::Mutation(mutation) if operator_semantic(mutation).is_some())
    {
        if parsed.cooperative.is_some() {
            return Err(invalid_request(
                "--operator cannot be combined with --cooperative-* caller selection: operator repair is attributed to the local user, never to a seat",
            ));
        }
        resolve_recipient_seats(&mut parsed, &paths, &connection, &clock)?;
        let CliAction::Mutation(mutation) = &parsed.action else {
            unreachable!()
        };
        let semantic = operator_semantic(mutation).expect("operator mutation selected above");
        return run_operator(semantic, &parsed, &paths, &clock, writer);
    }
    let seat_labels =
        if matches!(&parsed.action, CliAction::Wire(Command::Seats(_))) && output::human_active() {
            crate::host::native::NativeCli::new(context.host_endpoint.clone(), Arc::clone(&clock))
                .seat_labels(&budget())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
    let peer_source =
        crate::host::native::NativeCli::new(context.host_endpoint.clone(), Arc::clone(&clock));
    let Some(parsed) = peer_locations::with_source(peer_source, || {
        human::with_seat_labels(seat_labels, || {
            run_caller_scoped(
                parsed,
                caller_pane,
                &context,
                &paths,
                &connection,
                &clock,
                &budget,
                writer,
            )
        })
    })?
    else {
        return Ok(());
    };
    let max_bytes = match &parsed.action {
        CliAction::Wire(command) => command
            .page()
            .map_or(crate::protocol::pagination::MAX_PAGE_BYTES, |page| {
                page.max_bytes
            }),
        CliAction::PendingOps(page) => page.max_bytes,
        _ => crate::protocol::pagination::MAX_PAGE_BYTES,
    };
    let result = match parsed.action {
        CliAction::Wire(_) => unreachable!("wire reads run in run_caller_scoped"),
        CliAction::Daemon(DaemonAction::Health) => {
            let (_, _, client) = connect(&paths, &clock)?;
            client.call(Command::Health, &budget())?
        }
        CliAction::Daemon(DaemonAction::Ensure) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let descriptor = runtime.block_on(ensure_running(
                &context,
                &std::env::current_exe()?,
                Arc::clone(&clock),
            ))?;
            let client = LocalSocketClient::new(
                descriptor.endpoint,
                Arc::clone(&clock),
                descriptor.instance_uuid,
                Some(descriptor.boot_id),
            );
            client.call(Command::Health, &budget())?
        }
        CliAction::Daemon(DaemonAction::Stop) => {
            let (instance, descriptor) = published_endpoint(&paths)?;
            let client = LocalSocketClient::new(
                descriptor.endpoint.clone(),
                Arc::clone(&clock),
                instance,
                Some(descriptor.boot_id),
            );
            // The exit wait is an external wait (stretched in test children of
            // a loaded suite, external_bound); production keeps 5 s.
            let stop_budget = CallBudget {
                deadline: MonoInstant(
                    clock.monotonic_now().0.saturating_add(
                        crate::protocol::time::external_bound(std::time::Duration::from_secs(5))
                            .as_millis() as u64,
                    ),
                ),
                cancellation: Cancellation::default(),
            };
            stop_and_wait(&client, &paths, &descriptor, clock.as_ref(), &stop_budget)?;
            CommandResult::StopAccepted(StopAccepted {
                boot_id: descriptor.boot_id.to_string(),
            })
        }
        CliAction::PendingOps(page) => {
            let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
            let mut prefix = Vec::new();
            if let Some(state) = &parsed.output.context.state_dir {
                prefix.extend(["--state-dir".to_owned(), state.clone()]);
            }
            if let Some(host) = &parsed.output.context.host {
                prefix.extend(["--host-endpoint".to_owned(), host.as_str().to_owned()]);
            }
            if parsed.output.format == OutputFormat::Json {
                prefix.push("--json".to_owned());
            }
            prefix.push("pending-ops".to_owned());
            CommandResult::LocalIntents(journal.page_with_output(&page, &prefix, &parsed.output)?)
        }
        CliAction::Mutation(MutationSpec::Resolve(target)) => {
            let (instance, _, client) = connect(&paths, &clock)?;
            let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
            retry::run_new_api_to_writer_discarding_rejection(
                &journal,
                IntentScope::ServiceAllocation {
                    instance: instance.to_string(),
                    target: target.clone(),
                },
                SemanticMutation::ResolveSeat { target },
                clock.utc_now().0,
                || {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "ordinary resolution has no native claim",
                    ))
                },
                |command| client.call_with_output_definitive(command, &parsed.output, &budget()),
                &parsed.output,
                writer,
            )
            .map_err(retry_failure)?;
            return Ok(());
        }
        CliAction::Mutation(_) => {
            return Err(unsupported(
                "this mutation needs a caller seat or an explicit --operator form",
            ));
        }
        CliAction::Retry(recovery) => {
            // Retry needs the daemon; report it unavailable (exit 3, intent
            // kept) before inspecting the local journal.
            let (instance, _, client) = connect(&paths, &clock)?;
            let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
            let reference = journal.resolve_recovery_ref(recovery.as_str())?;
            let pending = journal.load(&reference)?;
            match (&pending.header.scope, &pending.semantic) {
                (
                    IntentScope::ServiceAllocation {
                        instance: recorded,
                        target,
                    },
                    SemanticMutation::ResolveSeat { target: requested },
                ) if recorded == &instance.to_string() && target == requested => {}
                (
                    IntentScope::Operator {
                        instance: recorded, ..
                    },
                    semantic,
                ) if recorded == &instance.to_string() && semantic.is_operator() => {}
                _ => {
                    return Err(unsupported(
                        "retry requires a matching ordinary resolution or operator intent; \
                         a seat's cooperative intent is retried from its pane or with \
                         --cooperative-* selection",
                    ));
                }
            }
            retry::run_retry_api_to_writer(
                &journal,
                &reference,
                &pending.header.scope,
                || {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "ordinary resolution has no native claim",
                    ))
                },
                |command| client.call_with_output(command, &parsed.output, &budget()),
                &parsed.output,
                writer,
            )
            .map_err(retry_failure)?;
            return Ok(());
        }
        CliAction::Doctor { .. } => {
            unreachable!("doctor is handled before context resolution")
        }
        CliAction::Setup(_) | CliAction::SetupAll(..) => {
            unreachable!("setup is handled before context resolution")
        }
        CliAction::Skill => unreachable!("skill is handled before context resolution"),
        CliAction::ContractId { .. } | CliAction::HarnessVersionNormalize { .. } => {
            unreachable!("contract-id and harness-version are handled before context resolution")
        }
        CliAction::InstallerIntegrations { .. } | CliAction::InternalJsonField { .. } => {
            unreachable!("internal json-field is handled before context resolution")
        }
        CliAction::Launch(_) | CliAction::Handoff(_) => {
            unreachable!("launch is handled after context resolution")
        }
        CliAction::MeInit { .. } => unreachable!("me init is handled after context resolution"),
        CliAction::Picker(_) => unreachable!("picker is handled after context resolution"),
        CliAction::Follow(_) => unreachable!("follow is handled after context resolution"),
        CliAction::Summary(_) => unreachable!("summary always derives a caller selection"),
        CliAction::View { once, page } => {
            if !once {
                return Err(unsupported(
                    "continuous operator view is unavailable; use `view --once`",
                ));
            }
            let (_, _, client) = connect(&paths, &clock)?;
            let bytes = render_view(
                &mut SocketViewReader(client),
                &page,
                &parsed.output,
                &budget(),
                clock.as_ref(),
            )?;
            writer.write_all(&bytes)?;
            writer.flush()?;
            return Ok(());
        }
        CliAction::CachedCheckIn(_) => {
            return Err(unsupported(
                "cached CheckIn requires the cooperative context provider",
            ));
        }
    };
    output::write_selected(&result, &parsed.output, max_bytes, writer)?;
    Ok(())
}

/// A daemon connection opened on first use and then shared, so one CLI
/// invocation never connects twice.
struct LazyConnection<C, F> {
    open: F,
    slot: std::cell::OnceCell<(uuid::Uuid, C)>,
}

impl<C, F: Fn() -> Result<(uuid::Uuid, C), RunError>> LazyConnection<C, F> {
    fn new(open: F) -> Self {
        Self {
            open,
            slot: std::cell::OnceCell::new(),
        }
    }

    fn get(&self) -> Result<&(uuid::Uuid, C), RunError> {
        if self.slot.get().is_none() {
            let opened = (self.open)()?;
            let _ = self.slot.set(opened);
        }
        self.slot.get().ok_or_else(daemon_unavailable)
    }
}

/// Derive the caller, then run the seat-acting action or the wire read on the
/// one shared connection. Returns the action back, unrun, when it is neither.
// Allowed: the parsed action and caller plus the runtime context (B5 agent
// evidence), paths, shared connection, clock, budget and writer.
#[allow(clippy::too_many_arguments)]
fn run_caller_scoped<C, F, W>(
    mut parsed: commands::ParsedCli,
    caller_pane: Option<&str>,
    context: &RuntimeContext,
    paths: &InstancePaths,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
    budget: &dyn Fn() -> CallBudget,
    writer: &mut W,
) -> Result<Option<commands::ParsedCli>, RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
    W: Write,
{
    if matches!(
        &parsed.action,
        CliAction::Wire(Command::Inbox(_) | Command::InboxBatch(_) | Command::InboxBatchV2(_))
    ) {
        // Pin effective routing even when it came from environment/defaults.
        parsed
            .output
            .context
            .state_dir
            .get_or_insert_with(|| context.state_dir.to_string_lossy().into_owned());
        parsed
            .output
            .context
            .host
            .get_or_insert_with(|| context.host_endpoint.to_string_lossy().into_owned());
    }
    // Capture omission before default_seat resolves the caller read. The second
    // guard below adds the canonical selected seat while retaining that choice.
    let _origin_inbox = output::InboxInvocationGuard::enter(&parsed, None);
    let selection = derive_caller(&mut parsed, caller_pane, context, paths, connection, clock)?;
    resolve_recipient_seats(&mut parsed, paths, connection, clock)?;
    let _inbox =
        output::InboxInvocationGuard::enter(&parsed, selection.as_ref().map(|s| s.seat.clone()));
    if let Some(selection) = selection {
        let (instance, client) = connection.get()?;
        run_selected(
            parsed,
            &selection,
            paths,
            *instance,
            client,
            clock.as_ref(),
            writer,
        )?;
        return Ok(None);
    }
    match parsed.action {
        CliAction::Wire(command) => {
            let (_, client) = connection.get()?;
            run_wire(command, &parsed.output, client, budget, writer)?;
            Ok(None)
        }
        action => {
            parsed.action = action;
            Ok(Some(parsed))
        }
    }
}

/// Run one wire read and write its result. The selected output travels with
/// the read, so the daemon fits the page to the bytes this caller sees and its
/// continuation commands keep this caller's format and selectors (a text read
/// must not hand back a `--json` continuation).
///
/// For a person at a terminal an inbox read makes one extra directory read
/// (the seat's threads, any membership) so the table can show each thread's
/// topic, which the wire inbox does not carry. Machine and JSON runs make no
/// extra call, and a failed directory read just leaves the topics out.
pub(crate) fn run_wire<C: LocalClient + ?Sized, W: Write>(
    command: Command,
    output_spec: &OutputSpec,
    client: &C,
    budget: &dyn Fn() -> CallBudget,
    writer: &mut W,
) -> Result<(), RunError> {
    let command = match command {
        Command::Inbox(query)
            // A continuation must stay in the protocol that captured it.
            if query.page.cursor.as_deref().is_none_or(|cursor| {
                cursor.starts_with(crate::protocol::pagination::INBOX_V2_PREFIX)
            }) && client
                .supports_capability(crate::protocol::capabilities::INBOX_BATCH_V2, &budget()) =>
        {
            Command::InboxBatchV2(query)
        }
        command => command,
    };
    let max_bytes = read_byte_bound(&command);
    let participant_thread = match &command {
        Command::Participants(query) => Some(query.thread.clone()),
        Command::Thread(query) => Some(query.thread.clone()),
        _ => None,
    };
    let inbox_seat = match &command {
        Command::Inbox(query) | Command::InboxBatchV2(query) => query.seat.clone(),
        _ => None,
    };
    // Service recovery deliberately accepts only the default wire output.
    // Its result has no server-generated continuations, so render the user's
    // selected text or JSON locally after the ordinary typed request returns.
    let result = if matches!(
        command,
        Command::ServiceInspect | Command::ServiceDisconnect(_)
    ) {
        client.call(command.clone(), &budget())?
    } else {
        client.call_with_output(command.clone(), output_spec, &budget())?
    };
    let topics = match (&result, inbox_seat) {
        (CommandResult::Inbox(page), Some(seat))
            if output::human_active()
                && output_spec.format == OutputFormat::Text
                && !page.items.is_empty() =>
        {
            inbox_topics(seat, output_spec, client, budget)
        }
        _ => None,
    };
    // Fitting and final rendering must share the exact topic snapshot: the
    // padded Human topic column is part of the caller's byte budget.
    let result = match &topics {
        Some(topics) => human::with_inbox_topics(topics.clone(), || {
            fit_inbox_read(command.clone(), result, output_spec, client, budget)
        }),
        None => fit_inbox_read(command.clone(), result, output_spec, client, budget),
    }?;
    let (result, modes) = fit_annotated_read(command, result, output_spec, client, budget)?;
    let peer_hints = if output_spec.format == OutputFormat::Text && peer_locations::active() {
        let location_budget = budget();
        peer_locations::prepare(
            &result,
            participant_thread,
            client,
            &location_budget,
            || peer_locations::snapshot(&location_budget),
        )
    } else {
        peer_locations::Prepared::default()
    };
    match topics {
        Some(topics) => {
            human::with_inbox_topics(topics, || {
                output::write_selected(&result, output_spec, max_bytes, writer)
            })?;
        }
        None => {
            let mut bytes = Vec::new();
            peer_locations::with_hints(&peer_hints, || {
                output::write_selected(&result, output_spec, max_bytes, &mut bytes)
            })?;
            if output_spec.format == OutputFormat::Text && !output::human_active() {
                bytes = peer_locations::append_compact(bytes, &peer_hints, max_bytes as usize);
            } else if peer_hints.unavailable && output_spec.format == OutputFormat::Text {
                bytes.extend_from_slice(b"location unavailable\n");
            }
            modes.write(bytes, max_bytes, writer)?;
        }
    };
    Ok(())
}

fn read_byte_bound(command: &Command) -> u32 {
    match command {
        Command::Message(q) => q.body.max_bytes,
        _ => command
            .page()
            .map_or(crate::protocol::pagination::MAX_PAGE_BYTES, |p| p.max_bytes),
    }
}

/// Refit server-selected chunks with the final CLI continuation overhead.
/// Only the effective request bound shrinks; emitted continuations retain the
/// original limit and byte bound through the invocation guard.
fn fit_inbox_read<C: LocalClient + ?Sized>(
    mut command: Command,
    mut result: CommandResult,
    spec: &OutputSpec,
    client: &C,
    budget: &dyn Fn() -> CallBudget,
) -> Result<CommandResult, RunError> {
    if !matches!(
        command,
        Command::Inbox(_) | Command::InboxBatch(_) | Command::InboxBatchV2(_)
    ) {
        return Ok(result);
    }
    let max = read_byte_bound(&command);
    loop {
        let bytes = output::emitted_bytes(&result, spec)?;
        if bytes.len() <= max as usize {
            return Ok(result);
        }
        // Human legacy rows can be much wider than their compact wire
        // encoding (especially UTF-8 topics). Reserve fewer canonical rows
        // rather than exhausting the wire byte budget without changing the
        // selected page. Its continuation remains service-generated.
        if output::human_active()
            && let CommandResult::Inbox(page) = &result
            && page.items.len() > 1
            && let Command::Inbox(query) = &mut command
        {
            let rows =
                (page.items.len() * max as usize / bytes.len()).clamp(1, page.items.len() - 1);
            query.page.limit = rows as u16;
            let retry = client.call_with_output(command.clone(), spec, &budget())?;
            if retry == result {
                return Err(ApiError::invalid_budget("inbox refit made no progress")
                    .with_required_minimum_bytes(bytes.len().try_into().unwrap_or(u32::MAX))
                    .into());
            }
            result = retry;
            continue;
        }
        let current = read_byte_bound(&command);
        let overflow = u32::try_from(bytes.len() - max as usize).unwrap_or(u32::MAX);
        let next = current.saturating_sub(overflow.max(current / 4));
        if next < 256 {
            return Err(ApiError::invalid_budget(
                "inbox continuation cannot fit selected byte budget",
            )
            .with_required_minimum_bytes(bytes.len().try_into().unwrap_or(u32::MAX))
            .into());
        }
        match &mut command {
            Command::Inbox(q) | Command::InboxBatch(q) | Command::InboxBatchV2(q) => {
                q.page.max_bytes = next
            }
            _ => unreachable!(),
        }
        let retry = client.call_with_output(command.clone(), spec, &budget())?;
        if retry == result {
            return Err(ApiError::invalid_budget("inbox refit made no progress").into());
        }
        result = retry;
    }
}

/// Reserve annotation bytes by reselecting the same read at a smaller bound.
/// The daemon, rather than the CLI, supplies any body/page continuation: never
/// trim printed body bytes or synthesize a cursor after selection.
fn fit_annotated_read<C: LocalClient + ?Sized>(
    mut command: Command,
    mut result: CommandResult,
    spec: &OutputSpec,
    client: &C,
    budget: &dyn Fn() -> CallBudget,
) -> Result<(CommandResult, output::ReadModes), RunError> {
    let max_bytes = read_byte_bound(&command);
    loop {
        let modes = output::ReadModes::lookup(&result, spec, client, &budget())?;
        if modes.is_empty() {
            return Ok((result, modes));
        }
        let len = modes.annotate(output::emitted_bytes(&result, spec)?).len();
        if len <= max_bytes as usize {
            return Ok((result, modes));
        }
        let current = read_byte_bound(&command);
        let overflow = u32::try_from(len - max_bytes as usize).unwrap_or(u32::MAX);
        let next = current.saturating_sub(overflow.max(current / 4));
        if next < 256 {
            return Err(RunError::Api(
                ApiError::invalid_budget("annotated read exceeds byte budget")
                    .with_required_minimum_bytes(len.try_into().unwrap_or(u32::MAX)),
            ));
        }
        match &mut command {
            Command::History(q) => q.page.max_bytes = next,
            Command::Search(q) => q.page.max_bytes = next,
            Command::Message(q) => q.body.max_bytes = next,
            _ => unreachable!("only selected message reads carry lazy markers"),
        }
        result = client.call_with_output(command.clone(), spec, &budget())?;
    }
}

const INBOX_DISPLAY_PAGE_READ_LIMIT: usize = 8;

fn select_display_inbox_page<C: LocalClient + ?Sized>(
    mut command: Command,
    output_spec: &OutputSpec,
    client: &C,
    clock: &dyn Clock,
) -> Result<(Command, CommandResult), RunError> {
    let selection_budget = cooperative_budget(clock);
    let mut result = client.call_with_output(command.clone(), output_spec, &selection_budget)?;
    // Source pages have their own bounded candidate walk. Retained, settled
    // history can fill that walk without producing anything to display. Keep
    // one bounded selection window, then present its actual continuation.
    for _ in 1..INBOX_DISPLAY_PAGE_READ_LIMIT {
        let (empty, stop_reason, has_more, next_cursor) = match &result {
            CommandResult::InboxBatch(page) => (
                page.items.is_empty(),
                page.stop_reason,
                page.has_more,
                &page.next_cursor,
            ),
            CommandResult::InboxBatchV2(page) => (
                page.items.is_empty(),
                page.stop_reason,
                page.has_more,
                &page.next_cursor,
            ),
            _ => break,
        };
        if !empty
            || stop_reason != crate::protocol::pagination::StopReason::Work
            || !has_more
            || selection_budget.is_exhausted(clock)
        {
            break;
        }
        let next = next_cursor.as_ref().ok_or_else(|| {
            RunError::Api(ApiError::store_corrupt(
                "inbox work page has no continuation",
            ))
        })?;
        let request = match &mut command {
            Command::InboxBatch(request) | Command::InboxBatchV2(request) => request,
            _ => unreachable!("display selection only reads compact inbox batches"),
        };
        if request.page.cursor.as_ref() == Some(next) {
            return Err(RunError::Api(ApiError::store_corrupt(
                "inbox work continuation did not advance",
            )));
        }
        request.page.cursor = Some(next.clone());
        result = client.call_with_output(command.clone(), output_spec, &selection_budget)?;
    }
    // Byte refitting must reselect this final source page, including any body
    // offset and frozen high waters in its request cursor, never the first page.
    Ok((command, result))
}

#[allow(clippy::too_many_arguments)]
fn run_display_inbox<C: LocalClient + ?Sized, W: Write>(
    query: &crate::protocol::commands::InboxQuery,
    output_spec: &OutputSpec,
    journal: &journal::Journal,
    contexts: &crate::harness::context::ContextJournal,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<(), RunError> {
    use crate::protocol::{ids::MessageId, results::InboxBatchItem};
    let context = contexts
        .current()
        .map_err(context_run_error)?
        .ok_or_else(|| {
            caller_not_located("context missing; explicit lifecycle check-in required")
        })?;
    let claim = crate::harness::bridge::caller_claim(&context).map_err(context_run_error)?;
    let mut request = query.clone();
    request.seat = Some(claim.seat.clone());
    let capabilities = match client.call(Command::Capabilities, &cooperative_budget(clock)) {
        Ok(CommandResult::Capabilities(list)) => list.capabilities,
        Ok(_) => Vec::new(),
        Err(error)
            if matches!(
                error.code,
                crate::protocol::results::ErrorCode::Unsupported
                    | crate::protocol::results::ErrorCode::InvalidRequest
            ) =>
        {
            Vec::new()
        }
        Err(error) => return Err(RunError::Api(error)),
    };
    let v2 = request
        .page
        .cursor
        .as_deref()
        .is_none_or(|cursor| cursor.starts_with(crate::protocol::pagination::INBOX_V2_PREFIX))
        && capabilities
            .iter()
            .any(|name| name == crate::protocol::capabilities::INBOX_BATCH_V2);
    let supported = capabilities
        .iter()
        .any(|name| name == crate::protocol::capabilities::INBOX_BATCH);
    if v2 {
        let (command, result) =
            select_display_inbox_page(Command::InboxBatchV2(request), output_spec, client, clock)?;
        let result = fit_inbox_read(command, result, output_spec, client, &|| {
            cooperative_budget(clock)
        })?;
        if !matches!(result, CommandResult::InboxBatchV2(_)) {
            return Err(RunError::Api(ApiError::store_corrupt(
                "daemon returned no v2 inbox batch",
            )));
        }
        let candidates = lazy_display::write_page(
            &result,
            output_spec,
            query.page.max_bytes,
            writer,
            journal,
            &claim,
            lazy_display::DisplaySelection::OwnDefaultText,
        )?;
        return lazy_display::settle(
            candidates,
            journal,
            clock.utc_now().0,
            |command| client.call(command, &cooperative_budget(clock)),
            output_spec,
        );
    }
    if !supported {
        return Err(unsupported(
            "this daemon does not support compact inbox display ACK; upgrade the daemon or use inbox --machine for a read-only view",
        ));
    }
    let (command, result) =
        select_display_inbox_page(Command::InboxBatch(request), output_spec, client, clock)?;
    let result = fit_inbox_read(command, result, output_spec, client, &|| {
        cooperative_budget(clock)
    })?;
    let CommandResult::InboxBatch(page) = &result else {
        return Err(RunError::Api(ApiError::store_corrupt(
            "daemon returned no inbox batch",
        )));
    };
    // A complete selected page must reach and flush the caller's output before
    // any claim that its messages were displayed can be submitted.
    output::write_selected(&result, output_spec, query.page.max_bytes, writer)?;
    if claim.harness == crate::protocol::authority::Harness::Human {
        return Ok(());
    }
    let mut candidates: Vec<MessageId> = Vec::new();
    for item in &page.items {
        if let InboxBatchItem::Message {
            message,
            body_start,
            body_end,
            body_len,
            ack_candidate,
            ..
        } = item
        {
            let complete_chain = journal.record_displayed_chunk(
                &claim,
                message,
                *body_start,
                *body_end,
                *body_len,
            )?;
            if complete_chain && ack_candidate.as_ref() == Some(message) {
                candidates.push(message.clone());
            }
        }
    }
    settle_displayed_ack(candidates, claim, output_spec, journal, client, clock)
}

fn settle_displayed_ack<C: LocalClient + ?Sized>(
    candidates: Vec<crate::protocol::ids::MessageId>,
    claim: crate::protocol::authority::CallerClaim,
    output_spec: &OutputSpec,
    journal: &journal::Journal,
    client: &C,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    if candidates.is_empty() {
        return Ok(());
    }
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    let displayed_claim = claim.clone();
    let settled_ids = candidates.clone();
    let semantic = SemanticMutation::freeze(
        SemanticMutation::AckDisplayed {
            messages: candidates,
        },
        claim,
    )?;
    let reference = journal.record(scope.clone(), semantic, clock.utc_now().0)?;
    let ack = retry::run_retry_api_to_writer(
        journal,
        &reference,
        &scope,
        || unreachable!("frozen inbox display claim"),
        |command| client.call(command, &cooperative_budget(clock)),
        output_spec,
        &mut io::sink(),
    );
    match ack {
        Ok(CommandResult::Acknowledged(_)) => {
            for message in &settled_ids {
                let _ = journal.clear_displayed_chunk(&displayed_claim, message);
            }
            Ok(())
        }
        Ok(_) => Err(RunError::Api(ApiError::store_corrupt(format!(
            "inbox page displayed; ACK result was unexpected; retry herdr-threads retry {}",
            reference.recovery_ref()
        )))),
        Err(retry::RetryFailure::Local(error)) => Err(RunError::Io(io::Error::new(
            error.kind(),
            format!(
                "inbox page displayed; ACK outcome pending: {error}; retry herdr-threads retry {}",
                reference.recovery_ref()
            ),
        ))),
        Err(retry::RetryFailure::Submit(mut error)) => {
            error.detail = format!(
                "inbox page displayed; ACK outcome pending: {}; retry herdr-threads retry {}",
                error.detail,
                reference.recovery_ref()
            );
            Err(RunError::Api(error))
        }
    }
}

/// `thread -> (topic, clipped)` for the seat's first directory page, or
/// `None` when the read fails.
fn inbox_topics<C: LocalClient + ?Sized>(
    seat: crate::protocol::ids::SeatId,
    output_spec: &OutputSpec,
    client: &C,
    budget: &dyn Fn() -> CallBudget,
) -> Option<std::collections::HashMap<crate::protocol::ids::ThreadId, (String, bool)>> {
    use crate::protocol::{
        commands::{DirectoryMembership, DirectoryQuery},
        pagination::{MAX_PAGE_BYTES, PageRequest},
    };
    let command = Command::Directory(DirectoryQuery {
        recent: false,
        membership: Some(seat),
        membership_filter: DirectoryMembership::All,
        topic_contains: None,
        page: PageRequest {
            cursor: None,
            limit: 50,
            max_bytes: MAX_PAGE_BYTES,
        },
    });
    match client.call_with_output(command, output_spec, &budget()) {
        Ok(CommandResult::Directory(page)) => Some(
            page.items
                .into_iter()
                .map(|summary| (summary.thread, (summary.topic_data, summary.topic_omitted)))
                .collect(),
        ),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_selected<C: LocalClient + ?Sized, W: Write>(
    parsed: commands::ParsedCli,
    selection: &CooperativeSelection,
    paths: &InstancePaths,
    instance: uuid::Uuid,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<(), RunError> {
    use crate::{
        harness::context::{OccupantContext, SessionReference},
        protocol::{commands::SeatInspectQuery, pagination::PageRequest, results::CommandResult},
    };
    if let CliAction::Retry(recovery) = &parsed.action {
        retry::preflight_original_actor(
            paths.instance_dir.join("intents"),
            recovery.as_str(),
            parsed.actor,
            &parsed.output.context,
        )?;
    } else {
        validate_actor_harness(parsed.actor, selection.harness)?;
    }
    if matches!(&parsed.action, CliAction::Summary(_)) {
        return summary::run(parsed, selection, paths, instance, client, clock, writer);
    }
    let request = Command::SeatInspect(SeatInspectQuery {
        seat: selection.seat.clone(),
        page: PageRequest {
            cursor: None,
            limit: 1,
            max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
        },
    });
    let result = client.call_with_output(request, &parsed.output, &cooperative_budget(clock))?;
    let CommandResult::SeatInspect(inspection) = result else {
        return Err(mapping_error("service returned no seat inspection"));
    };
    let generation = selected_generation(selection, &inspection)?;
    let contexts = seat_contexts(paths, instance, &selection.seat)?;
    let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
    let mut current = contexts.current().map_err(context_run_error)?;
    let lifecycle = matches!(
        &parsed.action,
        CliAction::Mutation(MutationSpec::CheckInLifecycle { .. })
    );
    // An agent's lifecycle check-in in a pane a person claimed with `me init`
    // replaces that person (a new binding generation). The reverse happens
    // only as the person's explicit `me init --operator` override
    // (TRUST-POLICY A4); the daemon audits it.
    let operator = matches!(
        &parsed.action,
        CliAction::Mutation(MutationSpec::CheckInLifecycle { operator: true, .. })
    );
    if let Some(context) = current.clone().filter(|context| {
        use crate::harness::context::Harness;
        lifecycle
            && context.harness == Harness::Human
            && selection.harness != Harness::Human
            && context.instance == instance
            && context.seat == selection.seat.as_str()
            && context.target == selection.target.as_str()
    }) {
        if !contexts
            .retire_current(&context)
            .map_err(context_run_error)?
        {
            return Err(mapping_error("local context changed during check-in"));
        }
        current = None;
    }
    // The person's `me init --operator` over an agent's context retires
    // nothing here: the daemon decides (TRUST-POLICY A4) and `dispatch`
    // replaces the local context only on success, so a rejection leaves the
    // agent's context intact. The agent context is treated as absent only for
    // seeding the check-in from the service mapping.
    if let Some(context) = &current
        && lifecycle
        && operator
        && context.harness != crate::harness::context::Harness::Human
        && selection.harness == crate::harness::context::Harness::Human
        && context.instance == instance
        && context.seat == selection.seat.as_str()
        && context.target == selection.target.as_str()
    {
        current = None;
    }
    // A fresh lifecycle check-in (a new event, not a replay of a recorded
    // one) is a deliberate new registration: it is seeded from the service's
    // current mapping and carries that generation as its exact CAS
    // (cooperative reports 1 and 10). Every other mutation, and a retried
    // check-in, must still match the local context or exactly replay.
    let mut fresh_lifecycle = false;
    if let Some(context) = &current {
        if context.instance != instance
            || context.seat != selection.seat.as_str()
            || context.target != selection.target.as_str()
            || context.harness != selection.harness
        {
            return Err(mapping_error(
                "local context differs from current service mapping",
            ));
        }
        if context.binding_generation != generation {
            let older = context.binding_generation < generation;
            let replay = older
                && exact_check_in_replay(&parsed.action, selection, instance, &journal, &contexts)?;
            // A recorded event that is not an exact replay stays bound to its
            // recorded request: `get_or_prepare` returns that request
            // unchanged, so it never re-seeds at the new generation.
            fresh_lifecycle = !replay && older && lifecycle;
            if !replay && !fresh_lifecycle {
                return Err(mapping_error(
                    "local context differs from current service mapping",
                ));
            }
        }
    }
    let compound = match &parsed.action {
        CliAction::Handoff(_) => true,
        CliAction::Retry(recovery) => {
            let reference = journal.resolve_recovery_ref(recovery.as_str())?;
            handoff::is_handoff(&journal.load(&reference)?.semantic)
        }
        _ => false,
    };
    if compound {
        if selection.role != crate::harness::context::Role::TopLevel {
            return Err(unsupported("handoff requires the top-level caller"));
        }
        let saved = current
            .as_ref()
            .ok_or_else(|| caller_not_located("handoff requires lifecycle check-in"))?;
        let claim = crate::harness::bridge::caller_claim(saved).map_err(context_run_error)?;
        return handoff::run(parsed, claim, &journal, paths, writer);
    }
    let initial = if (current.is_none() || fresh_lifecycle) && lifecycle {
        let execution = uuid::Uuid::new_v4();
        Some(OccupantContext {
            format_version: 1,
            instance,
            seat: selection.seat.as_str().into(),
            target: selection.target.as_str().into(),
            harness: selection.harness,
            binding_generation: generation,
            execution,
            session: SessionReference::PluginContext(execution),
            role: selection.role,
        })
    } else {
        None
    };
    run_cooperative(
        parsed,
        &journal,
        &contexts,
        initial.as_ref(),
        selection.role,
        client,
        clock,
        writer,
    )
}

/// Private per-seat cooperative context journal shared by explicit cooperative
/// CLI calls and native hook entrypoints.
pub(crate) fn seat_contexts(
    paths: &InstancePaths,
    instance: uuid::Uuid,
    seat: &crate::protocol::ids::SeatId,
) -> Result<crate::harness::context::ContextJournal, RunError> {
    crate::harness::context::ContextJournal::open(
        &seat_context_dir(paths, seat.as_str())?,
        instance,
        seat.as_str(),
        Duration::from_secs(1),
    )
    .map_err(context_run_error)
}

/// A completed predecessor may render its immutable result after its delivery
/// intent was removed. A pending predecessor must match both private journals
/// and still needs the store's exact-key replay; an uncommitted stale request
/// is rejected by the store at decision.
fn exact_check_in_replay(
    action: &CliAction,
    selection: &CooperativeSelection,
    instance: uuid::Uuid,
    journal: &journal::Journal,
    contexts: &crate::harness::context::ContextJournal,
) -> Result<bool, RunError> {
    use crate::harness::{
        bridge,
        context::{CheckInMode, ContextError, PendingCheckIn, SessionReference},
    };
    let matches_selection = |saved: &PendingCheckIn| {
        saved.mode == CheckInMode::Lifecycle
            && saved.context.instance == instance
            && saved.context.seat == selection.seat.as_str()
            && saved.context.target == selection.target.as_str()
            && saved.context.harness == selection.harness
            && saved.context.role == selection.role
    };
    let matches_native = |saved: &PendingCheckIn, supplied: Option<&str>| {
        (match &saved.context.session {
            SessionReference::Native(native) => Some(native.as_str()),
            SessionReference::PluginContext(_) => None,
        }) == supplied
    };
    let scope = IntentScope::Cooperative {
        instance: instance.to_string(),
        seat: selection.seat.clone(),
    };
    let (intent, supplied_native) = match action {
        CliAction::Mutation(MutationSpec::CheckInLifecycle {
            event_id,
            native_session,
            ..
        }) => {
            if let Some((saved, response)) = contexts
                .completed_for_event(event_id)
                .map_err(context_run_error)?
            {
                if !matches_selection(&saved) || !matches_native(&saved, native_session.as_deref())
                {
                    return Ok(false);
                }
                bridge::decode_request(&saved).map_err(context_run_error)?;
                let result: CommandResult = serde_json::from_slice(&response.output)
                    .map_err(|_| context_run_error(ContextError::Corrupt))?;
                let CommandResult::CheckedIn(check) = result else {
                    return Err(context_run_error(ContextError::Corrupt));
                };
                return Ok(check.context
                    == bridge::caller_claim(&response.context).map_err(context_run_error)?);
            }
            (
                journal.find_check_in(&scope, event_id)?,
                Some(native_session.as_deref()),
            )
        }
        CliAction::Retry(recovery) => {
            let reference = journal.resolve_recovery_ref(recovery.as_str())?;
            let pending = journal.load(&reference)?;
            if pending.header.scope == scope
                && matches!(
                    pending.semantic,
                    SemanticMutation::CooperativeCheckIn { .. }
                )
            {
                (Some(pending), None)
            } else {
                (None, None)
            }
        }
        _ => (None, None),
    };
    let Some(intent) = intent else {
        return Ok(false);
    };
    let SemanticMutation::CooperativeCheckIn { event_id, .. } = &intent.semantic else {
        return Ok(false);
    };
    let Some(saved) = contexts
        .request_for_event(event_id)
        .map_err(context_run_error)?
    else {
        return Ok(false);
    };
    if !matches_selection(&saved)
        || supplied_native.is_some_and(|native| !matches_native(&saved, native))
    {
        return Ok(false);
    }
    let frozen =
        bridge::pending_request(journal, &intent.header.reference).map_err(context_run_error)?;
    Ok(saved == frozen)
}

/// `claude` or `codex`, the names the contract and version helpers use.
fn harness_name(harness: crate::harness::context::Harness) -> &'static str {
    match harness {
        crate::harness::context::Harness::Codex => "codex",
        _ => "claude",
    }
}

fn invalid_request(detail: &str) -> RunError {
    RunError::Api(ApiError::invalid_request(detail))
}

fn context_error(error: io::Error) -> RunError {
    RunError::Api(ApiError::invalid_request(format!(
        "invalid state/host context: {error}"
    )))
}

/// The three documented administrative forms. Their authority is the kernel
/// peer UID the daemon observes on the private socket; nothing here asserts it.
fn operator_semantic(mutation: &MutationSpec) -> Option<SemanticMutation> {
    Some(match mutation {
        MutationSpec::FreshSeat(target) => SemanticMutation::OperatorFreshSeat {
            target: target.clone(),
        },
        MutationSpec::Retire(seat) => SemanticMutation::OperatorRetire { seat: seat.clone() },
        MutationSpec::Replace {
            seat,
            pane,
            replace,
        } => SemanticMutation::OperatorReplace {
            seat: seat.clone(),
            target: pane.clone(),
            replace: replace.clone(),
        },
        MutationSpec::Rebind { seat, pane } => SemanticMutation::OperatorRebind {
            seat: seat.clone(),
            target: pane.clone(),
        },
        MutationSpec::Invite {
            thread,
            seat,
            deadline_millis,
            operator: true,
        } => SemanticMutation::OperatorOrphanInvite {
            thread: thread.clone(),
            seat: seat.clone(),
            deadline_millis: *deadline_millis,
        },
        _ => return None,
    })
}

fn run_operator<W: Write>(
    semantic: SemanticMutation,
    parsed: &commands::ParsedCli,
    paths: &InstancePaths,
    clock: &Arc<dyn Clock>,
    writer: &mut W,
) -> Result<(), RunError> {
    if parsed.actor != actor_route::InvocationActor::Human {
        return Err(invalid_request(
            "operator actions require `herdr-threads human` before routing flags",
        ));
    }
    let (instance, _, client) = connect(paths, clock)?;
    let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
    retry::run_new_api_to_writer(
        &journal,
        IntentScope::Operator {
            instance: instance.to_string(),
            local_user_uid: crate::daemon::paths::effective_uid(),
        },
        semantic,
        clock.utc_now().0,
        || {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "operator actions carry no caller claim",
            ))
        },
        |command| {
            client.call_with_output(command, &parsed.output, &cooperative_budget(clock.as_ref()))
        },
        &parsed.output,
        writer,
    )
    .map_err(retry_failure)?;
    Ok(())
}

pub(crate) fn seat_context_dir(paths: &InstancePaths, seat: &str) -> Result<PathBuf, RunError> {
    paths.prepare_instance_dir()?;
    let contexts_root = paths.instance_dir.canonicalize()?.join("contexts");
    crate::daemon::paths::ensure_private_dir(&contexts_root)?;
    let context_dir = contexts_root.join(format!("{:x}", Sha256::digest(seat.as_bytes())));
    crate::daemon::paths::ensure_private_dir(&context_dir)?;
    Ok(context_dir)
}

const CALLER_HELP: &str = "run it inside the agent's own Herdr pane (HERDR_PANE_ID) after that \
     seat's lifecycle check-in (a person runs `herdr-threads me init` once in their own pane), or pass --cooperative-seat SEAT --cooperative-target PANE \
     --cooperative-harness codex|claude --cooperative-role top-level; operator repair uses the \
     explicit --operator forms and cannot send, accept, ACK or check in";

/// What an action needs from its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallerNeed {
    /// No seat: read-only, operator, ordinary resolution or local actions.
    /// Explicit `--cooperative-*` flags still select a seat when supplied.
    None,
    /// Acts as a seat (mutations, check-in, cached check-in, and `retry` of a
    /// seat's cooperative intent): needs a full cooperative selection.
    Selection,
    /// A read whose absent `--seat` defaults to the caller's seat. `required`
    /// is false when another explicit scope (a thread) keeps the read valid
    /// in a pane that maps to no seat.
    SeatDefault { required: bool },
    /// A thread read that marks the caller's own participant row when the
    /// invoking pane (or an explicit selection) locates a seat; best effort,
    /// never required.
    SelfMarker,
}

fn caller_need(
    parsed: &commands::ParsedCli,
    paths: &InstancePaths,
) -> Result<CallerNeed, RunError> {
    if parsed.caller_read_default && !matches!(parsed.action, CliAction::Wire(Command::Inbox(_))) {
        return Ok(CallerNeed::SeatDefault { required: true });
    }
    if !parsed.caller_read_default
        && matches!(parsed.action, CliAction::Wire(_))
        && parsed.pane_selector.is_some()
    {
        return Ok(CallerNeed::None);
    }
    Ok(match &parsed.action {
        CliAction::Mutation(mutation) => {
            if matches!(
                mutation,
                MutationSpec::Resolve(_)
                    | MutationSpec::Rebind { .. }
                    | MutationSpec::Retire(_)
                    | MutationSpec::Replace { .. }
                    | MutationSpec::FreshSeat(_)
                    | MutationSpec::Invite { operator: true, .. }
            ) {
                CallerNeed::None
            } else {
                CallerNeed::Selection
            }
        }
        CliAction::CachedCheckIn(_) | CliAction::Summary(_) | CliAction::Handoff(_) => {
            CallerNeed::Selection
        }
        CliAction::Retry(recovery) => {
            // Retry needs the daemon; report it unavailable (exit 3, intent
            // kept) before inspecting the local journal.
            published_endpoint(paths)?;
            let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
            let reference = journal.resolve_recovery_ref(recovery.as_str())?;
            if matches!(
                journal.load(&reference)?.header.scope,
                IntentScope::Cooperative { .. }
            ) {
                CallerNeed::Selection
            } else {
                CallerNeed::None
            }
        }
        CliAction::Wire(Command::Inbox(query))
            if query.seat.is_none()
                && parsed.output.format == OutputFormat::Text
                && parsed.presentation != output::Presentation::Machine =>
        {
            CallerNeed::Selection
        }
        CliAction::Wire(Command::Inbox(query) | Command::InboxBatchV2(query))
            if query.seat.is_none() =>
        {
            CallerNeed::SeatDefault { required: true }
        }
        CliAction::Wire(Command::PendingReceipts(query))
            if query.seat.is_none() && query.thread.is_none() =>
        {
            CallerNeed::SeatDefault { required: true }
        }
        CliAction::Wire(Command::Thread(_) | Command::Participants(_)) => CallerNeed::SelfMarker,
        _ => CallerNeed::None,
    })
}

fn default_seat(action: &mut CliAction, seat: crate::protocol::ids::SeatId) {
    match action {
        CliAction::Wire(Command::Inbox(query) | Command::InboxBatchV2(query)) => {
            query.seat = Some(seat)
        }
        CliAction::Wire(Command::PendingReceipts(query)) => query.seat = Some(seat),
        CliAction::Wire(Command::Directory(query)) => query.membership = Some(seat),
        CliAction::Wire(Command::Diagnostics(query)) => query.seat = Some(seat),
        CliAction::Wire(Command::Warnings(query)) => query.seat = seat,
        CliAction::Wire(Command::Thread(query)) => query.caller = Some(seat),
        CliAction::Wire(Command::Participants(query)) => query.caller = Some(seat),
        _ => {}
    }
}

/// Recipient pane selection never changes the caller's cooperative claim.
fn resolve_recipient_seats<C, F>(
    parsed: &mut commands::ParsedCli,
    paths: &InstancePaths,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
) -> Result<(), RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
{
    let recipient = matches!(
        parsed.action,
        CliAction::Mutation(MutationSpec::Invite { .. }) | CliAction::Wire(_)
    );
    let selector = recipient.then(|| parsed.pane_selector.clone()).flatten();
    if selector.is_none() && parsed.require_ack_panes.is_empty() {
        return Ok(());
    }
    let (instance, client) = connection.get()?;
    let allocate = |target: crate::protocol::ids::HostTargetId| -> Result<crate::protocol::ids::SeatId, RunError> {
        let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
        let (_, result) = retry::run_new_api_to_writer_discarding_rejection(
            &journal, IntentScope::ServiceAllocation { instance: instance.to_string(), target: target.clone() },
            SemanticMutation::ResolveSeat { target }, clock.utc_now().0,
            || Err(io::Error::new(io::ErrorKind::PermissionDenied, "recipient resolution has no caller claim")),
            |command| client.call_definitive(command, &cooperative_budget(clock.as_ref())),
            &parsed.output, &mut io::sink(),
        ).map_err(retry_failure)?;
        match result {
            CommandResult::SeatResolved(seat) => Ok(seat),
            _ => Err(mapping_error("service returned no resolved recipient seat")),
        }
    };
    if let Some(selector) = selector {
        let target = selector
            .direct_id()
            .ok_or_else(|| invalid_request("recipient pane was not frozen"))?;
        let seat = if matches!(
            parsed.action,
            CliAction::Mutation(MutationSpec::Invite { .. })
        ) {
            allocate(target)?
        } else {
            pane_seat(client, &parsed.output, &target, clock.as_ref())?
                .ok_or_else(|| no_seat_for_pane(&target))?
        };
        match &mut parsed.action {
            CliAction::Mutation(MutationSpec::Invite {
                seat: recipient, ..
            }) => *recipient = seat,
            CliAction::Wire(Command::Inbox(query) | Command::InboxBatchV2(query)) => {
                query.seat = Some(seat)
            }
            CliAction::Wire(Command::PendingReceipts(query)) => query.seat = Some(seat),
            CliAction::Wire(Command::Diagnostics(query)) => query.seat = Some(seat),
            CliAction::Wire(Command::Directory(query)) => query.membership = Some(seat),
            CliAction::Wire(Command::Warnings(query)) => query.seat = seat,
            _ => return Err(invalid_request("command has no recipient seat selector")),
        }
    }
    if recipient {
        parsed.pane_selector = None;
    }
    let mut seats = Vec::new();
    for selector in &parsed.require_ack_panes {
        let target = selector
            .direct_id()
            .ok_or_else(|| invalid_request("ACK recipient pane was not frozen"))?;
        let seat = allocate(target)?;
        if !seats.contains(&seat) {
            seats.push(seat);
        }
    }
    parsed.require_ack_panes.clear();
    if let CliAction::Mutation(MutationSpec::Send { require_ack, .. }) = &mut parsed.action {
        for seat in seats {
            if !require_ack.contains(&seat) {
                require_ack.push(seat);
            }
        }
    }
    Ok(())
}

/// The single caller derivation used by every CLI action. Explicit
/// `--cooperative-*` flags win. Otherwise seat-acting actions (including
/// `retry` of a cooperative intent) derive a full selection from the invoking
/// pane, and seat-defaulting reads (`inbox`, `pending-receipts`) fill their
/// absent `--seat` from the same pane mapping. This is cooperative locating,
/// not authentication: any process that inherits `HERDR_PANE_ID` derives the
/// same seat. It never allocates, accepts or ACKs; `run_selected` re-verifies
/// the mapping and context before a seat-acting action, and `run_cooperative`
/// rejects a retry whose intent belongs to another seat.
fn derive_caller<C, F>(
    parsed: &mut commands::ParsedCli,
    caller_pane: Option<&str>,
    context: &RuntimeContext,
    paths: &InstancePaths,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
) -> Result<Option<CooperativeSelection>, RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
{
    if let Some(selection) = &parsed.cooperative
        && !matches!(&parsed.action, CliAction::Retry(_))
    {
        validate_actor_harness(parsed.actor, selection.harness)?;
    }
    match caller_need(parsed, paths)? {
        CallerNeed::None => Ok(parsed.cooperative.clone()),
        CallerNeed::SelfMarker => {
            // Presentation only: any failure to locate the caller leaves the
            // rows unmarked, and the read itself reports daemon trouble.
            let seat = match (&parsed.cooperative, caller_pane.filter(|p| !p.is_empty())) {
                (Some(selection), _) => Some(selection.seat.clone()),
                (None, Some(pane)) => parse_pane(pane).ok().and_then(|pane| {
                    let (_, client) = connection.get().ok()?;
                    pane_seat(client, &parsed.output, &pane, clock.as_ref())
                        .ok()
                        .flatten()
                }),
                (None, None) => None,
            };
            if let Some(seat) = seat {
                default_seat(&mut parsed.action, seat);
            }
            Ok(parsed.cooperative.clone())
        }
        CallerNeed::Selection => match parsed.cooperative.clone() {
            Some(selection) => Ok(Some(selection)),
            None => {
                derive_selection(parsed, caller_pane, context, paths, connection, clock).map(Some)
            }
        },
        CallerNeed::SeatDefault { required } => {
            let seat = match (&parsed.cooperative, caller_pane.filter(|p| !p.is_empty())) {
                (Some(selection), _) => Some(selection.seat.clone()),
                (None, Some(pane)) => {
                    let pane = parse_pane(pane)?;
                    let (_, client) = connection.get()?;
                    match pane_seat(client, &parsed.output, &pane, clock.as_ref())? {
                        Some(seat) => Some(seat),
                        None if required => return Err(no_seat_for_pane(&pane)),
                        None => None,
                    }
                }
                (None, None) if required => {
                    return Err(caller_not_located(
                        "this read defaults to the caller's seat: pass --seat SEAT, or run it \
                         inside the agent's own Herdr pane (HERDR_PANE_ID)",
                    ));
                }
                (None, None) => None,
            };
            if let Some(seat) = seat {
                default_seat(&mut parsed.action, seat);
            }
            Ok(parsed.cooperative.clone())
        }
    }
}

fn parse_pane(pane: &str) -> Result<crate::protocol::ids::HostTargetId, RunError> {
    crate::protocol::ids::HostTargetId::parse(pane.to_owned())
        .map_err(|detail| invalid_request(&format!("HERDR_PANE_ID: {detail}")))
}

/// The one classification for every "no caller located" outcome: no
/// `HERDR_PANE_ID` for a seat-acting command or a seat-defaulting read, a
/// pane that maps to no resolved seat, and a mapped seat without a lifecycle
/// check-in context. Each is fixed by an argument or environment change
/// (`--cooperative-*`, `--seat`, the agent's own pane, or its check-in), so it
/// is `invalid_request`: exit 2, "invalid arguments or invalid local context"
/// in `commands::exit_status_help`. It is never `unsupported` (exit 4, which
/// callers read as "give up") nor a failed request (exit 1).
fn caller_not_located(detail: &str) -> RunError {
    invalid_request(detail)
}

fn no_seat_for_pane(pane: &crate::protocol::ids::HostTargetId) -> RunError {
    caller_not_located(&format!(
        "no resolved seat is mapped to pane {}; use `seat list`, `seat resolve --pane {0}` or \
         operator repair, then check in (a person in their own pane: `herdr-threads me init`); \
         reads may pass --seat SEAT",
        pane.as_str()
    ))
}

/// Nonretired seats mapped to one pane target, split by the one selection
/// rule the CLI and the hook share: a resolved seat always wins; an
/// unresolved seat is surfaced only when no resolved seat exists.
#[derive(Debug, Default)]
pub(crate) struct PaneSeats {
    pub resolved: Vec<crate::protocol::results::SeatSummary>,
    pub unresolved: Option<crate::protocol::results::SeatSummary>,
}

#[derive(Debug)]
pub(crate) enum PaneSeatsError {
    Api(ApiError),
    Unexpected,
}

/// Page size `collect_pane_seats` requests.
pub(crate) const PANE_SEAT_PAGE_LIMIT: u16 = 8;

/// Pages through the target-filtered seat query. The query orders by ordinal
/// (the cursor key), so an old unresolved seat can precede the resolved one and
/// there may be several of them; every page is read so none can truncate the
/// resolved seat away.
pub(crate) fn collect_pane_seats(
    pane: &crate::protocol::ids::HostTargetId,
    mut call: impl FnMut(Command) -> Result<CommandResult, ApiError>,
) -> Result<PaneSeats, PaneSeatsError> {
    use crate::protocol::{
        commands::SeatsQuery, pagination::PageRequest, results::ContinuityStatus,
    };
    const MAX_PAGES: usize = 64;
    let mut seats = PaneSeats::default();
    let mut cursor = None;
    for _ in 0..MAX_PAGES {
        let result = call(Command::Seats(SeatsQuery {
            page: PageRequest {
                cursor: cursor.take(),
                limit: PANE_SEAT_PAGE_LIMIT,
                max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
            },
            target: Some(pane.clone()),
            include_retired: false,
        }))
        .map_err(PaneSeatsError::Api)?;
        let CommandResult::Seats(page) = result else {
            return Err(PaneSeatsError::Unexpected);
        };
        for seat in page.items {
            if seat.target.as_ref() != Some(pane) || seat.retired_at.is_some() {
                continue;
            }
            if seat.continuity == ContinuityStatus::Resolved {
                seats.resolved.push(seat);
            } else if seats.unresolved.is_none() {
                seats.unresolved = Some(seat);
            }
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Ok(seats),
        }
    }
    Ok(seats)
}

/// The service's current resolved, nonretired mapping for `pane`: `None` when
/// no seat is mapped, an error when more than one is. Pages through every
/// target-filtered seat page (`collect_pane_seats`, the selection rule the hook
/// shares), so an older unresolved seat never truncates the resolved one and
/// two resolved seats are always seen.
fn pane_seat<C: LocalClient + ?Sized>(
    client: &C,
    output: &OutputSpec,
    pane: &crate::protocol::ids::HostTargetId,
    clock: &dyn Clock,
) -> Result<Option<crate::protocol::ids::SeatId>, RunError> {
    let seats = collect_pane_seats(pane, |command| {
        client.call_with_output(command, output, &cooperative_budget(clock))
    })
    .map_err(|error| match error {
        PaneSeatsError::Api(error) => RunError::from(error),
        PaneSeatsError::Unexpected => mapping_error("service returned no seat page"),
    })?;
    match seats.resolved.as_slice() {
        [one] => Ok(Some(one.seat.clone())),
        [] => Ok(None),
        _ => Err(mapping_error(&format!(
            "pane {} maps to more than one seat; pass --cooperative-seat (or --seat for reads) \
             explicitly",
            pane.as_str()
        ))),
    }
}

/// Full selection for a seat-acting action: the pane's current mapping picks
/// the seat and that seat's private lifecycle context supplies the recorded
/// harness and role. Nothing is inferred from the focused pane and no seat is
/// allocated.
fn derive_selection<C, F>(
    parsed: &commands::ParsedCli,
    caller_pane: Option<&str>,
    runtime: &RuntimeContext,
    paths: &InstancePaths,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
) -> Result<CooperativeSelection, RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
{
    let pane = caller_pane.filter(|pane| !pane.is_empty()).ok_or_else(|| {
        caller_not_located(&format!("this command acts as a seat: {CALLER_HELP}"))
    })?;
    let pane = parse_pane(pane)?;
    let (_, client) = connection.get()?;
    let seat = pane_seat(client, &parsed.output, &pane, clock.as_ref())?
        .ok_or_else(|| no_seat_for_pane(&pane))?;
    let instance = read_existing_namespace(paths)?.ok_or_else(daemon_unavailable)?;
    let contexts = seat_contexts(paths, instance, &seat)?;
    let context = match contexts.current().map_err(context_run_error)? {
        Some(context) => context,
        None => contexts
            .pending()
            .map_err(context_run_error)?
            .map(|pending| pending.context)
            .ok_or_else(|| {
                caller_not_located(&format!(
                    "seat {} on pane {} has no lifecycle check-in context yet; the launch driver \
                     or hook runs `check-in --lifecycle-event ID` with --cooperative-* selection; \
                     a person in their own pane runs `herdr-threads me init`",
                    seat.as_str(),
                    pane.as_str()
                ))
            })?,
    };
    if !matches!(&parsed.action, CliAction::Retry(_)) {
        validate_actor_harness(parsed.actor, context.harness)?;
    }
    let operator_override = contexts.operator_mark() == Some(context.execution);
    selection_from_context(
        pane,
        seat,
        &context,
        operator_override,
        std::env::vars(),
        |pane| pane_agent(runtime, clock, pane),
    )
}

/// Check the declared invocation route against the honestly selected harness.
/// Explicit Agent selection may replace an old Human context during lifecycle
/// check-in; this check therefore precedes, rather than inspects, that retirement.
fn validate_actor_harness(
    actor: actor_route::InvocationActor,
    harness: crate::harness::context::Harness,
) -> Result<(), RunError> {
    use crate::harness::context::Harness;
    use actor_route::InvocationActor;
    match (actor, harness) {
        (InvocationActor::Agent, Harness::Human) => Err(invalid_request(
            "this command selects a Human context; use `herdr-threads human` immediately after the executable, before routing flags",
        )),
        (InvocationActor::Human, Harness::Claude | Harness::Codex) => Err(invalid_request(
            "the human namespace cannot act through an agent cooperative selection; use the ordinary root command for that agent",
        )),
        _ => Ok(()),
    }
}

/// The selection a located seat's recorded context yields. A Human context
/// is refused when agent evidence is present (TRUST-POLICY A4, best effort;
/// the daemon's refusal is the guard) per `agent_evidence`, unless the person
/// recorded the `me init --operator` override for this context
/// (`operator_override`). A failed Herdr read does not refuse.
fn selection_from_context<K: AsRef<str>, V>(
    pane: crate::protocol::ids::HostTargetId,
    seat: crate::protocol::ids::SeatId,
    context: &crate::harness::context::OccupantContext,
    operator_override: bool,
    env: impl IntoIterator<Item = (K, V)>,
    read_agent: impl FnOnce(
        &crate::protocol::ids::HostTargetId,
    ) -> Result<Option<crate::ports::PaneAgentObservation>, ApiError>,
) -> Result<CooperativeSelection, RunError> {
    if context.target != pane.as_str() || context.seat != seat.as_str() {
        return Err(mapping_error(
            "local context differs from current service mapping",
        ));
    }
    if context.harness == crate::harness::context::Harness::Human
        && !operator_override
        && let Some(evidence) = agent_evidence(env, || read_agent(&pane))
    {
        return Err(agent_evidence_refusal(pane.as_str(), &evidence));
    }
    Ok(CooperativeSelection {
        seat,
        target: pane,
        harness: context.harness,
        role: context.role,
    })
}

/// Production composition of `launch`: the Herdr adapter on the explicit
/// host endpoint, the daemon's ordinary seat resolution, the owned setup
/// inspection for this state directory, and the private launch record.
fn run_launch<W: Write>(
    request: &launch::LaunchRequest,
    parsed: &commands::ParsedCli,
    context: &RuntimeContext,
    paths: &InstancePaths,
    clock: &Arc<dyn Clock>,
    writer: &mut W,
) -> Result<(), RunError> {
    if parsed.cooperative.is_some() {
        return Err(invalid_request(
            "launch is a local operator command; --cooperative-* caller selection does not apply",
        ));
    }
    let mut request = request.clone().with_process_options()?;
    let mut env = setup::SetupEnv::from_process(&parsed.output)?;
    env.state_dir = Some(context.state_dir.clone());
    env.host_endpoint = Some(context.host_endpoint.clone());
    let (instance, _, client) = connect(paths, clock)?;
    let journal = journal::Journal::open(paths.instance_dir.join("intents"))?;
    let host =
        crate::host::native::NativeCli::new(context.host_endpoint.clone(), Arc::clone(clock));
    // Without `--name`, the agent is named after the pane's Herdr label. Best
    // effort: an unreadable snapshot leaves the short-seat-id fallback.
    if request.name.is_none() {
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0.saturating_add(3_000)),
            cancellation: Cancellation::default(),
        };
        request.pane_label = host
            .pane_names(&budget)
            .ok()
            .and_then(|panes| launch::pane_label(&request.target, &panes));
    }
    let seats = launch::DaemonSeatResolver::new(&client, &journal, instance, clock.as_ref());
    let shell_probe = launch::SystemShellProbe::from_process();
    let outcome = launch::execute(
        &request,
        &launch::LaunchParts {
            env: &env,
            host: &host,
            seats: &seats,
            handoff: &client,
            clock: clock.as_ref(),
            record_dir: Some(&paths.instance_dir),
            shell_probe: &shell_probe,
        },
    )?;
    let bytes = match parsed.output.format {
        OutputFormat::Json => {
            let mut bytes = serde_json::to_vec(&serde_json::json!({ "launch": outcome.report }))
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            bytes.push(b'\n');
            bytes
        }
        OutputFormat::Text => setup::render_text(&outcome.report).into_bytes(),
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    if outcome.exit != 0 {
        return Err(RunError::Exit(outcome.exit));
    }
    Ok(())
}

fn retry_failure(failure: retry::RetryFailure<ApiError>) -> RunError {
    match failure {
        retry::RetryFailure::Local(error) => RunError::Io(error),
        retry::RetryFailure::Submit(error) => RunError::Api(error),
    }
}

#[cfg(test)]
#[path = "../../tests/cli/cooperative.rs"]
mod cooperative_tests;
#[cfg(test)]
#[path = "../../tests/cli/read_cost.rs"]
mod read_cost_tests;
#[cfg(test)]
#[path = "../../tests/cli/service_output.rs"]
mod service_output_tests;

/// Runtime composition seam for an already service-resolved durable seat and
/// private context directory. The provider declares honest role and harness;
/// native session labels alone must never choose or transfer a seat. The default
/// process runner remains unsupported until that mapping policy is supplied.
#[allow(clippy::too_many_arguments)]
pub fn run_cooperative<C: LocalClient + ?Sized, W: Write>(
    parsed: commands::ParsedCli,
    journal: &journal::Journal,
    contexts: &crate::harness::context::ContextJournal,
    initial: Option<&crate::harness::context::OccupantContext>,
    role: crate::harness::context::Role,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<(), RunError> {
    use crate::harness::{
        Capability, LifecycleEvent, bridge,
        context::{EventKind, Role},
    };
    let seat = if matches!(
        &parsed.action,
        CliAction::Wire(Command::Inbox(_) | Command::InboxBatch(_) | Command::InboxBatchV2(_))
    ) {
        contexts
            .current()
            .map_err(context_run_error)?
            .as_ref()
            .map(|c| crate::protocol::ids::SeatId::new(c.seat.clone()))
    } else {
        None
    };
    let _inbox = output::InboxInvocationGuard::enter(&parsed, seat);
    let _presentation = output::PresentationGuard::enter(parsed.presentation, &parsed.output);
    let own_text_inbox = parsed.caller_read_default
        && matches!(&parsed.action, CliAction::Wire(Command::Inbox(query)) if query.seat.is_none())
        && parsed.output.format == OutputFormat::Text
        && parsed.presentation != output::Presentation::Machine;
    if let CliAction::Retry(recovery) = &parsed.action {
        retry::preflight_original_actor(
            journal.root(),
            recovery.as_str(),
            parsed.actor,
            &parsed.output.context,
        )?;
    }
    if !matches!(&parsed.action, CliAction::Wire(_) | CliAction::Retry(_)) || own_text_inbox {
        let current = contexts.current().map_err(context_run_error)?;
        if let Some(context) = initial.or(current.as_ref()) {
            validate_actor_harness(parsed.actor, context.harness)?;
        }
    }
    if let CliAction::Wire(Command::Inbox(query)) = &parsed.action
        && own_text_inbox
        && role == Role::TopLevel
    {
        return run_display_inbox(
            query,
            &parsed.output,
            journal,
            contexts,
            client,
            clock,
            writer,
        );
    }
    if let CliAction::Wire(command) = parsed.action {
        return run_wire(
            command,
            &parsed.output,
            client,
            &|| cooperative_budget(clock),
            writer,
        );
    }
    if let CliAction::CachedCheckIn(request) = parsed.action {
        let saved = contexts
            .read_completed(&request.reference.key, &cooperative_budget(clock), clock)
            .map_err(crate::harness::cache::page_error)?;
        let page = crate::harness::cache::cached_output_page(&saved, &request, &parsed.output)?;
        output::write_selected(&page, &parsed.output, request.max_bytes, writer)?;
        return Ok(());
    }
    if role != Role::TopLevel {
        return Err(unsupported(
            "subagents may read and summarize; only top-level agents check in, accept or ACK",
        ));
    }
    let current = || {
        contexts
            .current()
            .map_err(context_run_error)?
            .ok_or_else(|| {
                caller_not_located("context missing; explicit lifecycle check-in required")
            })
    };
    match parsed.action {
        CliAction::Mutation(MutationSpec::CheckIn) => {
            let context = current()?;
            let event = LifecycleEvent {
                harness: context.harness,
                source: "current".into(),
                kind: EventKind::Tool,
                native_session: None,
                role,
                event_id: uuid::Uuid::new_v4().to_string(),
                capability: Capability::SourceSupported,
            };
            bridge::run_event(
                journal,
                contexts,
                &event,
                None,
                clock.utc_now().0,
                client,
                clock,
                &parsed.output,
                writer,
            )
            .map_err(bridge_run_error)?;
        }
        CliAction::Mutation(MutationSpec::CheckInLifecycle {
            event_id,
            native_session,
            operator,
        }) => {
            let context = contexts.current().map_err(context_run_error)?;
            let seed = initial
                .or(context.as_ref())
                .ok_or_else(|| unsupported("lifecycle requires a service-resolved durable seat"))?;
            let event = LifecycleEvent {
                harness: seed.harness,
                source: "explicit".into(),
                kind: EventKind::Startup,
                native_session,
                role,
                event_id,
                capability: Capability::SourceSupported,
            };
            bridge::run_event_as(
                journal,
                contexts,
                &event,
                initial,
                clock.utc_now().0,
                operator,
                client,
                clock,
                &parsed.output,
                writer,
            )
            .map_err(bridge_run_error)?;
        }
        CliAction::Mutation(mutation) => {
            let context = current()?;
            let claim = bridge::caller_claim(&context).map_err(context_run_error)?;
            let scope = IntentScope::Cooperative {
                instance: claim.instance.clone(),
                seat: claim.seat.clone(),
            };
            if matches!(&mutation, MutationSpec::Reject { .. }) {
                require_rejection_capability(client, clock)?;
            }
            let semantic = SemanticMutation::freeze(cooperative_semantic(mutation)?, claim)?;
            if semantic.is_lazy_send() {
                require_lazy_send_capability(client, clock)?;
            }
            retry::run_new_api_to_writer_discarding_rejection(
                journal,
                scope,
                semantic,
                clock.utc_now().0,
                || unreachable!("frozen cooperative claim"),
                |command| client.call_definitive(command, &cooperative_budget(clock)),
                &parsed.output,
                writer,
            )
            .map_err(retry_failure)?;
        }
        CliAction::Retry(recovery) => {
            let reference = journal.resolve_recovery_ref(recovery.as_str())?;
            let pending = journal.load(&reference)?;
            let seed = contexts
                .current()
                .map_err(context_run_error)?
                .or(contexts
                    .pending()
                    .map_err(context_run_error)?
                    .map(|p| p.context))
                .or_else(|| initial.cloned())
                .ok_or_else(|| caller_not_located("retry context missing"))?;
            let scope = IntentScope::Cooperative {
                instance: seed.instance.to_string(),
                seat: crate::protocol::ids::SeatId::parse(seed.seat)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            };
            if pending.header.scope != scope {
                return Err(unsupported("retry belongs to a different instance or seat"));
            }
            if let SemanticMutation::CooperativeCheckIn {
                claim,
                mode,
                event_id,
                operator,
            } = &pending.semantic
            {
                let saved_session = contexts
                    .request_for_event(event_id)
                    .map_err(context_run_error)?
                    .map(|request| match request.context.session {
                        crate::harness::context::SessionReference::Native(native) => Some(native),
                        crate::harness::context::SessionReference::PluginContext(_) => None,
                    });
                let event = LifecycleEvent {
                    harness: seed.harness,
                    source: "retry".into(),
                    kind: match mode {
                        crate::protocol::commands::CheckInMode::Current => EventKind::Tool,
                        _ => EventKind::Startup,
                    },
                    native_session: saved_session.unwrap_or_else(|| {
                        if claim.native_session.as_str().starts_with("plugin_context:") {
                            None
                        } else {
                            Some(claim.native_session.as_str().into())
                        }
                    }),
                    role,
                    event_id: event_id.clone(),
                    capability: Capability::SourceSupported,
                };
                bridge::run_event_as(
                    journal,
                    contexts,
                    &event,
                    initial,
                    clock.utc_now().0,
                    *operator,
                    client,
                    clock,
                    &parsed.output,
                    writer,
                )
                .map_err(bridge_run_error)?;
            } else {
                if pending.semantic.kind() == crate::protocol::results::IntentKind::Reject {
                    require_rejection_capability(client, clock)?;
                }
                if pending.semantic.is_lazy_send() {
                    require_lazy_send_capability(client, clock)?;
                }
                retry::run_retry_api_to_writer(
                    journal,
                    &reference,
                    &scope,
                    || unreachable!("frozen cooperative claim"),
                    |command| client.call(command, &cooperative_budget(clock)),
                    &parsed.output,
                    writer,
                )
                .map_err(retry_failure)?;
            }
        }
        _ => {
            return Err(unsupported(
                "local/operator commands require their separate runtime authority path",
            ));
        }
    }
    Ok(())
}
fn cooperative_budget(clock: &dyn Clock) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(5000)),
        cancellation: Cancellation::default(),
    }
}
fn context_run_error(error: crate::harness::context::ContextError) -> RunError {
    RunError::Io(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{error:?}"),
    ))
}
fn bridge_run_error(error: crate::harness::bridge::BridgeError) -> RunError {
    match error {
        crate::harness::bridge::BridgeError::Context(e) => context_run_error(e),
        crate::harness::bridge::BridgeError::Api(e) => RunError::Api(e),
        crate::harness::bridge::BridgeError::Io(e) => RunError::Io(e),
    }
}
fn cooperative_semantic(mutation: MutationSpec) -> io::Result<SemanticMutation> {
    Ok(match mutation {
        MutationSpec::Create { name, topic, goal } => {
            SemanticMutation::CreateThread { name, topic, goal }
        }
        MutationSpec::Name { thread, name } => SemanticMutation::SetThreadName { thread, name },
        MutationSpec::Topic { thread, topic } => SemanticMutation::SetTopic { thread, topic },
        MutationSpec::Invite {
            thread,
            seat,
            deadline_millis,
            operator: false,
        } => SemanticMutation::Invite {
            thread,
            seat,
            deadline_millis,
        },
        MutationSpec::Accept(thread) => SemanticMutation::Accept { thread },
        MutationSpec::Reject {
            thread,
            invitation,
            reason,
        } => SemanticMutation::Reject {
            thread,
            invitation,
            reason,
        },
        MutationSpec::AcceptRequired {
            thread,
            invitation,
            requirement,
            expected_revision,
        } => SemanticMutation::AcceptRequired {
            thread,
            invitation,
            requirement,
            expected_revision,
        },
        MutationSpec::Leave(thread) => SemanticMutation::Leave { thread },
        MutationSpec::Send {
            delivery_mode,
            thread,
            body,
            require_ack,
            deadline_millis,
            relays_user,
            user_intent,
        } => SemanticMutation::SendMessage {
            delivery_mode,
            thread,
            body,
            invited_recipients: require_ack,
            deadline_millis,
            relays_user,
            user_intent,
        },
        MutationSpec::Ack(messages) => SemanticMutation::Ack { messages },
        MutationSpec::Archive(thread) => SemanticMutation::Archive { thread },
        MutationSpec::Reopen(thread) => SemanticMutation::Reopen { thread },
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "operator and allocation authority require separate runtime paths",
            ));
        }
    })
}

/// Socket adapter preserves selected text/JSON fitting at the transport boundary.
/// Runtime may inject it into `run_cooperative` or the harness bridge.
pub struct SelectedSocketClient<'a> {
    pub client: &'a LocalSocketClient,
    pub output: &'a OutputSpec,
}
impl LocalClient for SelectedSocketClient<'_> {
    fn supports_capability(&self, name: &str, budget: &CallBudget) -> bool {
        self.client.supports_capability(name, budget)
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        if matches!(
            command,
            Command::ServiceInspect | Command::ServiceDisconnect(_)
        ) {
            self.client.call(command, budget)
        } else {
            self.client.call_with_output(command, self.output, budget)
        }
    }
    fn call_definitive(
        &self,
        command: Command,
        budget: &CallBudget,
    ) -> Result<Result<CommandResult, ApiError>, ApiError> {
        self.client
            .call_with_output_definitive(command, self.output, budget)
    }
    fn call_with_output(
        &self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if output != self.output {
            return Err(ApiError::invalid_request("selected output mismatch"));
        }
        self.client.call_with_output(command, output, budget)
    }
}

/// Older daemons are refused before any rejection intent is submitted.
fn require_rejection_capability<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    match client.call(Command::Capabilities, &cooperative_budget(clock)) {
        Ok(CommandResult::Capabilities(list))
            if list
                .capabilities
                .iter()
                .any(|name| name == crate::protocol::capabilities::INVITATION_REJECT) =>
        {
            Ok(())
        }
        Err(error)
            if !matches!(
                error.code,
                crate::protocol::results::ErrorCode::Unsupported
                    | crate::protocol::results::ErrorCode::InvalidRequest
            ) =>
        {
            Err(RunError::Api(error))
        }
        _ => Err(unsupported(
            "daemon lacks invitation.reject_v1; use a compatible daemon before rejecting invitations",
        )),
    }
}

/// Refuse unsupported lazy delivery before publishing an intent or replaying it.
fn require_lazy_send_capability<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    match client.call(Command::Capabilities, &cooperative_budget(clock)) {
        Ok(CommandResult::Capabilities(list))
            if list
                .capabilities
                .iter()
                .any(|name| name == crate::protocol::capabilities::LAZY_SEND) =>
        {
            Ok(())
        }
        Err(error)
            if !matches!(
                error.code,
                crate::protocol::results::ErrorCode::Unsupported
                    | crate::protocol::results::ErrorCode::InvalidRequest
            ) =>
        {
            Err(RunError::Api(error))
        }
        _ => Err(unsupported(
            "daemon lacks send.lazy_v1; upgrade the daemon before sending lazy messages",
        )),
    }
}
