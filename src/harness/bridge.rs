//! Cooperative hook bridge. The launch driver resolves a durable seat first and
//! supplies its private journal/initial seed; session labels never allocate seats.
//! `LocalClient` implementations must honor the supplied five-second budget.
use super::{
    LifecycleEvent,
    cache::{CachedCheckIn, cache_reference},
    context::*,
};
use crate::{
    cli::journal::{IntentRef, IntentScope, Journal, SemanticMutation},
    ports::LocalClient,
    protocol::{
        attention::{AttentionDigest, AttentionToken},
        authority::{CallerClaim, CallerRole, Harness as WireHarness},
        commands::{CheckInMode as WireMode, Command, DirectoryMembership, DirectoryQuery},
        ids::*,
        output::{OutputFormat, OutputSpec, encode_selected},
        pagination::{
            Consistency, MAX_CURSOR_BYTES, MAX_PAGE_BYTES, Page, PageRequest, StopReason,
        },
        results::{ApiError, CommandResult},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};
use std::io::{self, Write};
use uuid::Uuid;

#[derive(Debug)]
pub enum BridgeError {
    Context(ContextError),
    Api(ApiError),
    Io(io::Error),
}
impl From<ContextError> for BridgeError {
    fn from(e: ContextError) -> Self {
        Self::Context(e)
    }
}
impl From<io::Error> for BridgeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Debug)]
pub enum PresentationError {
    Invalid,
    TooLarge,
    Api(ApiError),
}
impl From<PresentationError> for BridgeError {
    fn from(error: PresentationError) -> Self {
        match error {
            PresentationError::Invalid => Self::Context(ContextError::Invalid),
            PresentationError::TooLarge => Self::Context(ContextError::TooLarge),
            PresentationError::Api(error) => Self::Api(error),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewReason {
    Lifecycle,
    Recovery,
    None,
}

pub enum OriginalOffer<'a> {
    None,
    Inline(&'a [u8]),
    Reference(&'a [String]),
}

pub enum OverviewPresentation<'a> {
    NotRequested,
    Directory(&'a CommandResult),
    Failed(&'a [String]),
}

pub struct HookInput<'a> {
    pub instruction: &'a str,
    pub original: OriginalOffer<'a>,
    pub overview: OverviewPresentation<'a>,
    pub output: &'a OutputSpec,
    pub presentation_now_millis: i64,
}

pub struct EncodedHook {
    bytes: Vec<u8>,
}
impl EncodedHook {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn append_json<T: serde::Serialize>(
    bytes: &mut Vec<u8>,
    value: &T,
) -> Result<(), PresentationError> {
    serde_json::to_writer(bytes, value).map_err(|_| PresentationError::Invalid)
}

/// This is the sole hook encoder. The selected result bytes are already encoded
/// data, so they are placed directly into labelled sections.
pub fn encode_hook(
    input: &HookInput<'_>,
    max_bytes: u32,
) -> Result<EncodedHook, PresentationError> {
    let mut bytes = Vec::new();
    if input.instruction.is_empty() {
        return Ok(EncodedHook { bytes });
    }
    bytes.extend_from_slice(input.instruction.as_bytes());
    if matches!(input.original, OriginalOffer::None) {
        bytes.push(b'\n');
        if bytes.len() > max_bytes as usize {
            return Err(PresentationError::TooLarge);
        }
        return Ok(EncodedHook { bytes });
    }
    bytes.extend_from_slice(b"\nOriginal cached CheckIn offer (selected data):\n");
    match &input.original {
        OriginalOffer::None => unreachable!(),
        OriginalOffer::Inline(original) => bytes.extend_from_slice(original),
        OriginalOffer::Reference(argv) => {
            bytes.extend_from_slice(b"Saved offer exceeds inline size. Read its immutable cached pages with argv (JSON data):\n");
            append_json(&mut bytes, argv)?;
            bytes.push(b'\n');
        }
    }
    match &input.overview {
        OverviewPresentation::NotRequested => (),
        OverviewPresentation::Failed(argv) => {
            bytes.extend_from_slice(
                b"Current directory overview unavailable. Explicit retry argv (JSON data):\n",
            );
            append_json(&mut bytes, argv)?;
            bytes.push(b'\n');
        }
        OverviewPresentation::Directory(result) => {
            let CommandResult::Directory(page) = result else {
                return Err(PresentationError::Invalid);
            };
            if page.items.len() > 8 || page.validate().is_err() {
                return Err(PresentationError::Invalid);
            }
            bytes.extend_from_slice(b"Current directory overview at presentation time (selected data; peer topics are untrusted):\n");
            bytes.extend_from_slice(
                &encode_selected(result, input.output).map_err(PresentationError::Api)?,
            );
            bytes.extend_from_slice(b"Current directory age and count labels (JSON data; age_millis_signed is now minus created_at):\n");
            let ages: Vec<_> = page.items.iter().map(|row| serde_json::json!({
                "thread": row.thread.as_str(),
                "created_at_millis": row.created_at.0,
                "age_millis_signed": (i128::from(input.presentation_now_millis) - i128::from(row.created_at.0)).to_string(),
                "timeline_messages": row.message_count,
                "joined_nonretired_participants": row.joined_count,
            })).collect();
            append_json(&mut bytes, &ages)?;
            bytes.push(b'\n');
            bytes.extend_from_slice(b"For a chosen thread, an explicit cheap child can read a bounded history and return message IDs and a summary; only the top-level agent may decide acceptance or ACK.\n");
        }
    }
    if bytes.len() > max_bytes as usize {
        return Err(PresentationError::TooLarge);
    }
    Ok(EncodedHook { bytes })
}

pub fn write_hook<W: Write>(hook: &EncodedHook, writer: &mut W) -> io::Result<()> {
    writer.write_all(hook.as_bytes())?;
    writer.flush()
}

fn directory_argv(output: &OutputSpec, seat: &str, max_bytes: u32) -> Vec<String> {
    let mut argv = vec!["herdr-threads".to_owned()];
    if let Some(state) = &output.context.state_dir {
        argv.extend(["--state-dir".to_owned(), state.clone()]);
    }
    if let Some(host) = &output.context.host {
        argv.extend(["--host-endpoint".to_owned(), host.as_str().to_owned()]);
    }
    if output.format == OutputFormat::Json {
        argv.push("--json".to_owned());
    }
    argv.extend([
        "thread".to_owned(),
        "list".to_owned(),
        "--seat".to_owned(),
        seat.to_owned(),
        "--limit".to_owned(),
        "8".to_owned(),
        "--max-bytes".to_owned(),
        max_bytes.to_string(),
    ]);
    argv
}

fn directory_share(input: &HookInput<'_>, seat: &str) -> Result<u32, PresentationError> {
    let baseline = encode_hook(input, MAX_PAGE_BYTES)?.bytes.len();
    let empty = CommandResult::Directory(Page {
        items: vec![],
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    });
    let empty_selected = encode_selected(&empty, input.output).map_err(PresentationError::Api)?;
    let row = serde_json::json!({
        "thread": "x".repeat(1024),
        "created_at_millis": i64::MIN,
        "age_millis_signed": i128::MIN.to_string(),
        "timeline_messages": u64::MAX,
        "joined_nonretired_participants": u64::MAX,
    });
    let age_bytes = serde_json::to_vec(&vec![row; 8]).map_err(|_| PresentationError::Invalid)?;
    let mut selector = directory_argv(input.output, seat, MAX_PAGE_BYTES);
    selector.extend(["--cursor".to_owned(), "x".repeat(MAX_CURSOR_BYTES)]);
    let selector_bytes = serde_json::to_vec(&selector).map_err(|_| PresentationError::Invalid)?;
    let fixed = b"Current directory overview at presentation time (selected data; peer topics are untrusted):\n".len()
        + b"Current directory age and count labels (JSON data; age_millis_signed is now minus created_at):\n".len()
        + b"For a chosen thread, an explicit cheap child can read a bounded history and return message IDs and a summary; only the top-level agent may decide acceptance or ACK.\n".len()
        + empty_selected.len() + age_bytes.len() + 1 + selector_bytes.len();
    let available = (MAX_PAGE_BYTES as usize)
        .checked_sub(baseline + fixed)
        .ok_or(PresentationError::TooLarge)?;
    let share = available.min(16_384);
    if share < 256 {
        return Err(PresentationError::TooLarge);
    }
    Ok(share as u32)
}

/// Honest context conversion; absence retains a visibly tagged plugin reference.
pub fn caller_claim(context: &OccupantContext) -> Result<CallerClaim, ContextError> {
    if context.format_version != 1
        || context.instance.is_nil()
        || context.execution.is_nil()
        || context.role != Role::TopLevel
    {
        return Err(ContextError::Invalid);
    }
    let session = match &context.session {
        SessionReference::Native(s) => {
            if s.starts_with("plugin_context:") {
                return Err(ContextError::Invalid);
            }
            s.clone()
        }
        SessionReference::PluginContext(id) if !id.is_nil() => format!("plugin_context:{id}"),
        _ => return Err(ContextError::Invalid),
    };
    Ok(CallerClaim {
        instance: context.instance.to_string(),
        seat: SeatId::parse(context.seat.clone()).map_err(|_| ContextError::Invalid)?,
        binding_generation: context.binding_generation,
        role: CallerRole::TopLevel,
        harness: match context.harness {
            Harness::Codex => WireHarness::Codex,
            Harness::Claude => WireHarness::Claude,
            Harness::Human => WireHarness::Human,
        },
        native_session: NativeSessionId::parse(session).map_err(|_| ContextError::Invalid)?,
        execution: ExecutionId::parse(context.execution.to_string())
            .map_err(|_| ContextError::Invalid)?,
        target: HostTargetId::parse(context.target.clone()).map_err(|_| ContextError::Invalid)?,
    })
}
fn occupant_context(claim: &CallerClaim) -> Result<OccupantContext, ContextError> {
    if claim.role != CallerRole::TopLevel {
        return Err(ContextError::Child);
    }
    let session = if let Some(id) = claim
        .native_session
        .as_str()
        .strip_prefix("plugin_context:")
    {
        SessionReference::PluginContext(Uuid::parse_str(id).map_err(|_| ContextError::Invalid)?)
    } else {
        SessionReference::Native(claim.native_session.as_str().into())
    };
    Ok(OccupantContext {
        format_version: 1,
        instance: Uuid::parse_str(&claim.instance).map_err(|_| ContextError::Invalid)?,
        seat: claim.seat.as_str().into(),
        target: claim.target.as_str().into(),
        harness: match claim.harness {
            WireHarness::Codex => Harness::Codex,
            WireHarness::Claude => Harness::Claude,
            WireHarness::Human => Harness::Human,
        },
        binding_generation: claim.binding_generation,
        execution: Uuid::parse_str(claim.execution.as_str()).map_err(|_| ContextError::Invalid)?,
        session,
        role: Role::TopLevel,
    })
}
pub(crate) fn pending_request(
    journal: &Journal,
    reference: &IntentRef,
) -> Result<PendingCheckIn, ContextError> {
    let intent = journal.load(reference)?;
    let SemanticMutation::CooperativeCheckIn {
        claim,
        mode,
        event_id,
    } = &intent.semantic
    else {
        return Err(ContextError::Invalid);
    };
    let context = occupant_context(claim)?;
    let (mode, expected_generation) = match mode {
        WireMode::Current => (CheckInMode::Current, None),
        WireMode::Lifecycle {
            expected_binding_generation,
        } => (CheckInMode::Lifecycle, Some(*expected_binding_generation)),
    };
    let command = intent
        .semantic
        .to_command(reference.operation.clone(), None)?;
    Ok(PendingCheckIn {
        operation_id: Uuid::parse_str(reference.operation.as_str())
            .map_err(|_| ContextError::Invalid)?,
        mode,
        context,
        expected_generation,
        event_id: event_id.clone(),
        payload_version: 1,
        payload: serde_json::to_vec(&command).map_err(|_| ContextError::Invalid)?,
    })
}
/// Called once per stable external lifecycle event; concurrent duplicates and
/// restarts reuse the first published CLI key, execution, claim and CAS.
/// Initial seed is service-resolved at its current generation and is valid only
/// for lifecycle. The service rechecks the frozen generation at decision.
pub fn prepare_event(
    journal: &Journal,
    contexts: &ContextJournal,
    event: &LifecycleEvent,
    initial: Option<&OccupantContext>,
    created_at_millis: i64,
) -> Result<Option<PendingCheckIn>, ContextError> {
    if !event.can_check_in() {
        return Ok(None);
    }
    let request = contexts.get_or_prepare(&event.event_id, |current| {
        // A lifecycle event seeded by the caller's service-resolved mapping
        // (`initial`) registers at that mapping's generation even when an
        // older local context exists; `Current` mode only continues the local
        // context. An existing request for this event is returned unchanged
        // by `get_or_prepare`, so a retry never re-seeds.
        let seed = if event.kind.mode() == CheckInMode::Lifecycle {
            initial.or(current)
        } else {
            current
        }
        .ok_or(ContextError::LifecycleRequired)?;
        if seed.harness != event.harness || seed.role != Role::TopLevel {
            return Err(ContextError::Conflict);
        }
        if let Some(native) = &event.native_session
            && event.kind.mode() == CheckInMode::Current
            && seed.session != SessionReference::Native(native.clone())
        {
            return Err(ContextError::Conflict);
        }
        let scope = IntentScope::Cooperative {
            instance: seed.instance.to_string(),
            seat: SeatId::parse(seed.seat.clone()).map_err(|_| ContextError::Invalid)?,
        };
        let reference =
            journal.record_check_in(scope, &event.event_id, created_at_millis, || {
                let next = seed
                    .for_event(event.kind, Uuid::new_v4(), event.native_session.clone())
                    .map_err(context_io)?;
                let mode = match event.kind.mode() {
                    CheckInMode::Current => WireMode::Current,
                    CheckInMode::Lifecycle => WireMode::Lifecycle {
                        expected_binding_generation: seed.binding_generation,
                    },
                };
                Ok((caller_claim(&next).map_err(context_io)?, mode))
            })?;
        pending_request(journal, &reference)
    })?;
    validate_event(&request, event)?;
    Ok(Some(request))
}
fn validate_event(request: &PendingCheckIn, event: &LifecycleEvent) -> Result<(), ContextError> {
    if request.event_id != event.event_id
        || request.context.harness != event.harness
        || request.context.role != event.role
        || request.mode != event.kind.mode()
    {
        return Err(ContextError::Conflict);
    }
    if request.mode == CheckInMode::Lifecycle {
        let recorded = match &request.context.session {
            SessionReference::Native(native) => Some(native.as_str()),
            SessionReference::PluginContext(_) => None,
        };
        if recorded != event.native_session.as_deref() {
            return Err(ContextError::Conflict);
        }
    } else if let Some(native) = &event.native_session
        && request.context.session != SessionReference::Native(native.clone())
    {
        return Err(ContextError::Conflict);
    }
    Ok(())
}
/// Reject edited serialized payload or mismatch between the two durable journals.
pub fn decode_request(request: &PendingCheckIn) -> Result<Command, ContextError> {
    if request.payload_version != 1 {
        return Err(ContextError::Invalid);
    }
    let command: Command =
        serde_json::from_slice(&request.payload).map_err(|_| ContextError::Corrupt)?;
    let Command::CheckIn(check) = &command else {
        return Err(ContextError::Invalid);
    };
    let mode = match request.mode {
        CheckInMode::Current if request.expected_generation.is_none() => WireMode::Current,
        CheckInMode::Lifecycle => WireMode::Lifecycle {
            expected_binding_generation: request
                .expected_generation
                .ok_or(ContextError::Invalid)?,
        },
        _ => return Err(ContextError::Invalid),
    };
    if check.claim != caller_claim(&request.context)?
        || check.operation.as_str() != request.operation_id.to_string()
        || check.mode != mode
    {
        return Err(ContextError::Conflict);
    }
    command.validate().map_err(|_| ContextError::Invalid)?;
    Ok(command)
}
fn context_io(error: ContextError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}"))
}

#[allow(clippy::too_many_arguments)]
fn cached_event<C: LocalClient + ?Sized>(
    journal: &Journal,
    contexts: &ContextJournal,
    event: &LifecycleEvent,
    initial: Option<&OccupantContext>,
    created_at_millis: i64,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
) -> Result<Option<(PendingCheckIn, CheckInResponse, CommandResult)>, BridgeError> {
    let Some(request) = prepare_event(journal, contexts, event, initial, created_at_millis)? else {
        return Ok(None);
    };
    let mut api_error = None;
    let response = contexts.dispatch(&event.event_id, &mut |pending: &PendingCheckIn| {
        let command = decode_request(pending)?;
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0.saturating_add(5000)),
            cancellation: Cancellation::default(),
        };
        let result = client
            .call_with_output(command, output, &budget)
            .map_err(|error| {
                let detail = error.detail.clone();
                api_error = Some(error);
                ContextError::Dispatch(detail)
            })?;
        let CommandResult::CheckedIn(check) = &result else {
            return Err(ContextError::Invalid);
        };
        Ok(CheckInResponse {
            context: occupant_context(&check.context)?,
            historical: check.context_disposition
                == crate::protocol::results::CheckInContextDisposition::Historical,
            output: serde_json::to_vec(&result).map_err(|_| ContextError::Invalid)?,
        })
    });
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            return Err(match api_error {
                Some(error) => BridgeError::Api(error),
                None => BridgeError::Context(error),
            });
        }
    };
    let result: CommandResult =
        serde_json::from_slice(&response.output).map_err(|_| ContextError::Corrupt)?;
    Ok(Some((request, response, result)))
}

/// CheckIn only. Cache response/context before presentation; intent completion is
/// deferred until the selected output reaches and flushes through the writer.
/// An ended historical response never replaces a local successor context.
// Allowed: one lifecycle event plus the journals, client, clock and output it is bridged through.
#[allow(clippy::too_many_arguments)]
pub fn run_event<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    contexts: &ContextJournal,
    event: &LifecycleEvent,
    initial: Option<&OccupantContext>,
    created_at_millis: i64,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<Option<CommandResult>, BridgeError> {
    let Some((request, _response, result)) = cached_event(
        journal,
        contexts,
        event,
        initial,
        created_at_millis,
        client,
        clock,
        output,
    )?
    else {
        return Ok(None);
    };
    crate::cli::output::write_selected(
        &result,
        output,
        crate::protocol::pagination::MAX_PAGE_BYTES,
        writer,
    )
    .map_err(|error| match error {
        crate::cli::output::OutputError::Api(e) => BridgeError::Api(e),
        crate::cli::output::OutputError::Io(e) => BridgeError::Io(e),
    })?;
    // Completed duplicate hooks may have already removed this CLI intent.
    journal.complete_operation(&OperationId::new(request.operation_id.to_string()))?;
    Ok(Some(result))
}

/// Hook presentation carries fixed receipt instructions and the bounded compact
/// CheckIn offer. No history/body RPC, acceptance or ACK occurs here.
#[allow(clippy::too_many_arguments)]
pub fn run_hook_event<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    contexts: &ContextJournal,
    event: &LifecycleEvent,
    initial: Option<&OccupantContext>,
    created_at_millis: i64,
    client: &C,
    clock: &dyn Clock,
    writer: &mut W,
) -> Result<(), BridgeError> {
    let reason = if event.kind.mode() == CheckInMode::Lifecycle {
        OverviewReason::Lifecycle
    } else {
        OverviewReason::None
    };
    run_hook_event_with_reason(
        journal,
        contexts,
        event,
        initial,
        created_at_millis,
        client,
        clock,
        &OutputSpec::default(),
        reason,
        writer,
    )
}

/// A qualified adapter may request Recovery explicitly. The reason changes only
/// presentation, never the persisted CheckIn request or event identity.
#[allow(clippy::too_many_arguments)]
pub fn run_hook_event_with_reason<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    contexts: &ContextJournal,
    event: &LifecycleEvent,
    initial: Option<&OccupantContext>,
    created_at_millis: i64,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    reason: OverviewReason,
    writer: &mut W,
) -> Result<(), BridgeError> {
    run_hook_event_reporting_notices(
        journal,
        contexts,
        event,
        initial,
        created_at_millis,
        client,
        clock,
        output,
        reason,
        writer,
        &mut None,
    )
}

/// What a presented check-in carried, for the native hook's budgeted
/// rendering: the programmatic notice page the offer settled and the startup
/// directory overview in compact per-thread rows.
#[derive(Debug, Clone, Default)]
pub struct Presented {
    pub notices: crate::protocol::results::NoticeOffer,
    pub overview: Option<super::OverviewRows>,
}

/// `run_hook_event_with_reason`, also reporting what the offer presented in
/// `presented`: the programmatic notice page it carried (and so settled), so
/// the hook can show it on the `offered notices:` line that survives the
/// oversize fallback, and the directory overview, so the fallback trims it per
/// thread instead of dropping it whole.
#[allow(clippy::too_many_arguments)]
pub fn run_hook_event_reporting_notices<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    contexts: &ContextJournal,
    event: &LifecycleEvent,
    initial: Option<&OccupantContext>,
    created_at_millis: i64,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    reason: OverviewReason,
    writer: &mut W,
    presented: &mut Option<Presented>,
) -> Result<(), BridgeError> {
    let reason = if event.kind.mode() == CheckInMode::Lifecycle {
        OverviewReason::Lifecycle
    } else {
        match reason {
            OverviewReason::Lifecycle => return Err(ContextError::Invalid.into()),
            other => other,
        }
    };
    let instruction = super::render_context(event.role, &[], true)?;
    if !event.can_check_in() {
        let hook = encode_hook(
            &HookInput {
                instruction: &instruction,
                original: OriginalOffer::None,
                overview: OverviewPresentation::NotRequested,
                output,
                presentation_now_millis: clock.utc_now().0,
            },
            MAX_PAGE_BYTES,
        )?;
        write_hook(&hook, writer)?;
        return Ok(());
    }
    let Some((request, response, result)) = cached_event(
        journal,
        contexts,
        event,
        initial,
        created_at_millis,
        client,
        clock,
        output,
    )?
    else {
        return Ok(());
    };
    let CommandResult::CheckedIn(check) = &result else {
        return Err(ContextError::Invalid.into());
    };
    *presented = Some(Presented {
        notices: check.notices.clone(),
        overview: None,
    });
    // The store always reports its offer frontier; with nothing to offer that
    // frontier alone is routine no-change output.
    let quiet_current = event.kind.mode() == CheckInMode::Current
        && reason == OverviewReason::None
        && check.warning_count == 0
        && !check.warning_count_has_more
        && check.warnings.items.is_empty()
        && !check.warnings.has_more
        && check.notices.items.is_empty()
        && !check.notices.has_more
        && check.inbox.items.is_empty()
        && !check.inbox.has_more;
    if quiet_current {
        let hook = EncodedHook { bytes: Vec::new() };
        write_hook(&hook, writer)?;
        journal.complete_operation(&OperationId::new(request.operation_id.to_string()))?;
        return Ok(());
    }
    let selected = encode_selected(&result, output).map_err(BridgeError::Api)?;
    let reference_argv;
    let original = if selected.len() <= 16_384 {
        OriginalOffer::Inline(&selected)
    } else {
        let saved = CachedCheckIn {
            request: request.clone(),
            response: response.clone(),
        };
        let reference =
            cache_reference(&saved, &output.context).map_err(|_| PresentationError::Invalid)?;
        let token = reference.token().map_err(|_| PresentationError::Invalid)?;
        reference_argv = {
            let mut argv = vec!["herdr-threads".to_owned()];
            if let Some(state) = &output.context.state_dir {
                argv.extend(["--state-dir".to_owned(), state.clone()]);
            }
            if let Some(host) = &output.context.host {
                argv.extend(["--host-endpoint".to_owned(), host.as_str().to_owned()]);
            }
            if output.format == OutputFormat::Json {
                argv.push("--json".to_owned());
            }
            argv.extend([
                "cached-check-in".to_owned(),
                "--reference".to_owned(),
                token,
                "--max-bytes".to_owned(),
                MAX_PAGE_BYTES.to_string(),
            ]);
            argv
        };
        OriginalOffer::Reference(&reference_argv)
    };
    let base = HookInput {
        instruction: &instruction,
        original,
        overview: OverviewPresentation::NotRequested,
        output,
        presentation_now_millis: clock.utc_now().0,
    };
    let overview = if reason == OverviewReason::None {
        None
    } else {
        let share = directory_share(&base, &request.context.seat)?;
        let retry_argv = directory_argv(output, &request.context.seat, share);
        let command = Command::Directory(DirectoryQuery {
            membership: Some(
                SeatId::parse(request.context.seat.clone()).map_err(|_| ContextError::Invalid)?,
            ),
            membership_filter: DirectoryMembership::Default,
            topic_contains: None,
            page: PageRequest {
                cursor: None,
                limit: 8,
                max_bytes: share,
            },
        });
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0.saturating_add(5000)),
            cancellation: Cancellation::default(),
        };
        match client.call_with_output(command, output, &budget) {
            Ok(result @ CommandResult::Directory(_)) => Some(result),
            Ok(_) => return Err(ContextError::Invalid.into()),
            Err(error) => {
                let diagnostic = encode_hook(
                    &HookInput {
                        overview: OverviewPresentation::Failed(&retry_argv),
                        ..base
                    },
                    MAX_PAGE_BYTES,
                )?;
                write_hook(&diagnostic, writer)?;
                return Err(BridgeError::Api(error));
            }
        }
    };
    if let (Some(presented), Some(CommandResult::Directory(page))) =
        (presented.as_mut(), overview.as_ref())
    {
        presented.overview = Some(super::OverviewRows::from_directory(
            page,
            base.presentation_now_millis,
        ));
    }
    let hook = encode_hook(
        &HookInput {
            overview: match &overview {
                Some(result) => OverviewPresentation::Directory(result),
                None => OverviewPresentation::NotRequested,
            },
            ..base
        },
        MAX_PAGE_BYTES,
    )?;
    write_hook(&hook, writer)?;
    journal.complete_operation(&OperationId::new(request.operation_id.to_string()))?;
    Ok(())
}

/// Read the seat's server-side attention digest (root adoption, wave-1 fix2
/// (a)): one read-only, seat-scoped query whose token is complete by
/// construction. The hook compares only that token with its per-execution
/// mark; nothing here pages the inbox or reconstructs a frontier.
pub fn read_digest<C: LocalClient + ?Sized>(
    client: &C,
    seat: &SeatId,
    budget: &CallBudget,
) -> Result<AttentionDigest, ApiError> {
    let result = client.call(
        Command::AttentionDigest(crate::protocol::commands::AttentionDigestQuery {
            seat: seat.clone(),
        }),
        budget,
    )?;
    let invalid = |detail: &str| ApiError {
        code: crate::protocol::results::ErrorCode::StoreCorrupt,
        detail: detail.into(),
        restart_argv: None,
        required_minimum_bytes: None,
    };
    let CommandResult::AttentionDigest(digest) = result else {
        return Err(invalid("service returned no attention digest"));
    };
    digest.validate().map_err(invalid)?;
    if &digest.seat != seat {
        return Err(invalid("attention digest for another seat"));
    }
    Ok(digest)
}

/// Seed for the tool-boundary mark from a completed lifecycle offer. `run`
/// performs the lifecycle CheckIn; the digest is read strictly before it when
/// that CheckIn is fresh, so the seeded token never covers a publication the
/// offer did not. A replayed (already completed) event's offer predates any
/// read now and is not seeded; a failed digest read is not seeded either (the
/// next tool call then re-presents, never suppresses).
pub fn seeded_lifecycle<C: LocalClient + ?Sized, E>(
    contexts: &ContextJournal,
    event_id: &str,
    client: &C,
    seat: &SeatId,
    budget: &CallBudget,
    run: impl FnOnce() -> Result<(), E>,
) -> Result<Option<(Uuid, AttentionDigest)>, E> {
    let fresh = matches!(contexts.completed_for_event(event_id), Ok(None));
    let digest = if fresh {
        read_digest(client, seat, budget).ok()
    } else {
        None
    };
    run()?;
    Ok(contexts
        .completed_for_event(event_id)
        .ok()
        .flatten()
        .filter(|(_, response)| !response.historical)
        .and_then(|(_, response)| Some((response.context.execution, digest?))))
}

/// Result of a tool-boundary read. `mark` is committed by the caller only after
/// `text` has been delivered, so a lost write can never suppress an offer.
/// `summary` is the digest line, kept separate so an oversized offer's
/// fallback can still carry it.
#[derive(Debug, Default)]
pub struct ToolBoundary {
    pub text: Vec<u8>,
    pub summary: Option<String>,
    /// The digest read before the offer, for the ready-to-run command block.
    pub digest: Option<AttentionDigest>,
    pub mark: Option<(Uuid, AttentionToken)>,
}

/// Non-durable tool-boundary attention read (harness design: tool context is
/// single-invocation scoped and excluded from durable client intent).
///
/// One read-only digest query first. When its token has not advanced beyond
/// this execution's mark the call is quiet and no CheckIn is made. Otherwise
/// one Current CheckIn under the registered context's cooperative fences, with
/// a fresh operation key, is presented with the digest summary; nothing is
/// written to the context or intent journal and nothing is replayed. The new
/// mark is the join of the old mark and the token read *before* the offer, so
/// a publication arriving after that read always advances the next call. A
/// digest failure fails open: the offer is presented and the mark is kept.
/// `coalesce: false` (a compaction) re-presents any pending attention.
pub fn tool_boundary_check_in<C: LocalClient + ?Sized>(
    contexts: &ContextJournal,
    event: &LifecycleEvent,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    budget: &CallBudget,
    coalesce: bool,
) -> Result<ToolBoundary, BridgeError> {
    if event.kind.mode() != CheckInMode::Current || !event.can_check_in() {
        return Err(ContextError::Invalid.into());
    }
    let saved = contexts.current()?.ok_or(ContextError::LifecycleRequired)?;
    if saved.harness != event.harness
        || event
            .native_session
            .as_ref()
            .is_some_and(|native| saved.session != SessionReference::Native(native.clone()))
    {
        return Err(ContextError::LifecycleRequired.into());
    }
    let claim = caller_claim(&saved)?;
    let stored = contexts.attention_mark(saved.execution);
    let last = if coalesce { stored } else { None };
    let digest = read_digest(client, &claim.seat, budget);
    let mut mark = None;
    if let Ok(digest) = &digest {
        let advanced = match &last {
            Some(last) => digest.token.advanced_beyond(last),
            // No mark (first call, or a compaction): present what is pending.
            None => !digest.is_empty(),
        };
        // The stored mark only grows, whatever the coalescing decision.
        let next = stored.map_or(digest.token, |stored| stored.join(&digest.token));
        if !advanced {
            return Ok(ToolBoundary {
                text: Vec::new(),
                summary: None,
                digest: None,
                mark: (stored != Some(next)).then_some((saved.execution, next)),
            });
        }
        mark = Some((saved.execution, next));
    }
    let command = Command::CheckIn(crate::protocol::commands::CheckIn {
        mode: WireMode::Current,
        claim,
        operation: OperationId::new(Uuid::new_v4().to_string()),
    });
    let result = client
        .call_with_output(command, output, budget)
        .map_err(BridgeError::Api)?;
    let CommandResult::CheckedIn(check) = &result else {
        return Err(ContextError::Invalid.into());
    };
    if check.context_disposition == crate::protocol::results::CheckInContextDisposition::Historical
    {
        // The registered execution was superseded; only a lifecycle event may
        // register again.
        return Err(ContextError::LifecycleRequired.into());
    }
    let instruction = super::render_context(event.role, &[], true)?;
    let selected = encode_selected(&result, output).map_err(BridgeError::Api)?;
    let hook = encode_hook(
        &HookInput {
            instruction: &instruction,
            original: OriginalOffer::Inline(&selected),
            overview: OverviewPresentation::NotRequested,
            output,
            presentation_now_millis: clock.utc_now().0,
        },
        MAX_PAGE_BYTES,
    )?;
    // The carried notice page rides with the digest summary, which survives
    // the hook's oversize fallback: the offer settles exactly what it shows.
    let digest = digest.ok();
    Ok(ToolBoundary {
        text: hook.bytes,
        summary: join_summaries(
            digest.as_ref().map(AttentionDigest::summary),
            check.notices.summary(),
        ),
        digest,
        mark,
    })
}

/// The digest summary and the carried notice line, one per line.
pub fn join_summaries(digest: Option<String>, notices: Option<String>) -> Option<String> {
    match (digest, notices) {
        (Some(digest), Some(notices)) => Some(format!("{digest}\n{notices}")),
        (digest, notices) => digest.or(notices),
    }
}
