//! Hidden `watch` and `watch ack` subcommands: a push-channel mod's delivery child.
//! The contract (argv, env, exit codes, JSON lines) is in
//! [`crate::protocol::watch`]; this file is the CLI behaviour (spec D3, D4, D6).
//!
//! `watch` never ACKs, enrolls, allocates or rebinds. After the daemon accepts
//! the registration it streams bounded schema-1 JSON lines of everything
//! pending for the seat (once after `Accepted`, then after every `Attention`
//! frame) and exits per the D3 table. `watch ack` reports mod deliveries.

use super::{
    LazyConnection, RunError, cooperative_budget, derive_caller, published_endpoint, seat_contexts,
};
use crate::{
    delivery::{
        Assembler, DeliveryKind, DrainEvent, DrainPagesError, MAX_DRAIN_PAGES, PAGE_MAX_BYTES,
        PAGE_MAX_ITEMS, PageError, PageSource, Stop, drain_pages, read_truncated_hint,
    },
    harness::{bridge, context::Harness},
    ports::LocalClient,
    protocol::{
        authority::CallerClaim,
        capabilities::MOD_WATCH,
        commands::{AckModDelivered, Command, InboxQuery},
        ids::{MessageId, NativeSessionId, OperationId, SeatId},
        pagination::{Page, PageRequest},
        results::{ApiError, CommandResult, ErrorCode, InboxBatchV2Item},
        time::{CallBudget, Clock},
        watch::{
            ModAckItem, ModAckOutcome, ModAckReason, ModDeliveryVia, WATCH_EXIT_ERROR,
            WATCH_EXIT_STREAM_ENDED, WatchAttention, WatchAttentionCleared, WatchFrame, WatchItem,
            WatchLine, WatchOutcome, WatchReply, WatchRequest as WireWatch, WatchStatus,
            WatchStatusReason, WatchStatusState, WatchWireRequest,
        },
        wire::{PROTOCOL_VERSION, WireResponse},
    },
};
use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// Read timeout of the stream; each expiry is one parent-liveness check.
const STREAM_READ_TIMEOUT: Duration = Duration::from_secs(1);
/// Budget for the registration round trip.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRequest {
    pub harness: Harness,
    pub session: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchAckRequest {
    pub session: String,
    pub via: ModDeliveryVia,
    pub messages: Vec<MessageId>,
}

/// Process-wide emit bookkeeping: the status counter and the delivery
/// assembler (ids already streamed by this process, the attention state, the
/// truncation hint file and the marker's command prefix).
#[derive(Debug, Default)]
pub(crate) struct EmitState {
    next_status: u64,
    assembler: Assembler,
}

impl EmitState {
    fn new(hint_path: Option<PathBuf>) -> Self {
        Self {
            assembler: Assembler::new(hint_path),
            ..Self::default()
        }
    }

    fn with_command_prefix(mut self, prefix: Vec<String>) -> Self {
        self.assembler = self.assembler.with_command_prefix(prefix);
        self
    }
}

#[derive(Debug)]
pub(crate) enum DrainError {
    /// The daemon refused or failed a read.
    Api,
    /// The daemon's pages broke the paging contract.
    Protocol,
    /// stdout is gone (EPIPE): the parent stopped reading.
    Out,
}

impl From<ApiError> for DrainError {
    fn from(_: ApiError) -> Self {
        Self::Api
    }
}

impl From<io::Error> for DrainError {
    fn from(_: io::Error) -> Self {
        Self::Out
    }
}

fn write_line(out: &mut dyn Write, line: &WatchLine) -> io::Result<()> {
    let mut text = serde_json::to_vec(line).map_err(io::Error::other)?;
    text.push(b'\n');
    out.write_all(&text)?;
    out.flush()
}

fn emit_status(
    state: &mut EmitState,
    out: &mut dyn Write,
    status: WatchStatus,
) -> Result<(), io::Error> {
    let id = format!("status:{}", state.next_status);
    state.next_status += 1;
    write_line(out, &WatchLine::new(id, WatchItem::Status(status)))
}

/// One status line for a refusal or CLI-local reason; returns the exit code.
fn refuse(state: &mut EmitState, out: &mut dyn Write, reason: WatchStatusReason) -> i32 {
    let exit = reason.exit_code();
    // A closed stdout ends the process quietly with 0, like every other write.
    if emit_status(
        state,
        out,
        WatchStatus {
            state: WatchStatusState::Refused,
            reason: Some(reason),
            exit: Some(exit),
        },
    )
    .is_err()
    {
        return 0;
    }
    exit
}

fn exit_result(code: i32) -> Result<(), RunError> {
    if code == 0 {
        Ok(())
    } else {
        Err(RunError::Exit(code))
    }
}

/// The checks that run before any connection or context read, in order:
/// `HERDR_THREADS_MOD_DELIVERY=off` (both `watch` and `watch ack`), then, for
/// `watch` only, a missing pane without `--cooperative-*` selection.
pub(crate) fn precheck(
    mod_delivery: Option<&str>,
    caller_pane: Option<&str>,
    cooperative: bool,
    is_watch: bool,
    out: &mut dyn Write,
) -> Result<(), RunError> {
    let mut state = EmitState::default();
    if mod_delivery == Some("off") {
        return exit_result(refuse(&mut state, out, WatchStatusReason::EnvDisabled));
    }
    if is_watch && !cooperative && caller_pane.is_none_or(str::is_empty) {
        return exit_result(refuse(&mut state, out, WatchStatusReason::NoPane));
    }
    Ok(())
}

/// Parent liveness: the parent pid captured at start is still ours and is not
/// init. `current_ppid` is injected so tests never fork.
fn parent_alive(start_ppid: i32, current_ppid: impl FnOnce() -> i32) -> bool {
    let now = current_ppid();
    now == start_ppid && now != 1
}

fn system_ppid() -> i32 {
    // SAFETY: getppid has no preconditions and cannot fail.
    unsafe { libc::getppid() }
}

/// Source of daemon frames after registration.
pub(crate) trait WatchConn {
    /// `Ok(Some)`: a frame. `Ok(None)`: the stream ended. `Err` of kind
    /// `TimedOut`/`WouldBlock`: nothing arrived within `timeout`.
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<WatchFrame>, io::Error>;
}

/// Blocking length-prefixed frame reader over a Unix stream. Bytes read before
/// a timeout stay buffered, so a partial frame is never lost.
struct SocketConn {
    stream: UnixStream,
    buf: Vec<u8>,
}

impl SocketConn {
    fn take_frame(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let length = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
        let length = length as usize;
        if length == 0 || length > crate::daemon::transport::MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid frame length",
            ));
        }
        if self.buf.len() < 4 + length {
            return Ok(None);
        }
        let frame = self.buf[4..4 + length].to_vec();
        self.buf.drain(..4 + length);
        Ok(Some(frame))
    }

    /// `Ok(None)`: clean end of stream between frames.
    fn read_frame(&mut self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        // macOS rejects setsockopt (EINVAL) once the peer has closed; the read
        // below then reports the end of stream at once, so the error is moot.
        let _ = self
            .stream
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))));
        loop {
            if let Some(frame) = self.take_frame()? {
                return Ok(Some(frame));
            }
            let mut chunk = [0u8; 8192];
            match self.stream.read(&mut chunk) {
                Ok(0) if self.buf.is_empty() => return Ok(None),
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl WatchConn for SocketConn {
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<WatchFrame>, io::Error> {
        match self.read_frame(timeout)? {
            None => Ok(None),
            Some(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        }
    }
}

enum OpenError {
    Unavailable,
    Unsupported,
    Other,
}

/// Connect, send the registration frame and read the reply.
fn open_watch(
    socket: &Path,
    instance: uuid::Uuid,
    claim: CallerClaim,
) -> Result<(WatchReply, SocketConn), OpenError> {
    let mut stream = UnixStream::connect(socket).map_err(|_| OpenError::Unavailable)?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let request = WatchWireRequest {
        version: PROTOCOL_VERSION,
        request_id: request_id.clone(),
        expected_instance: instance.to_string(),
        expected_boot: None,
        watch: WireWatch { claim },
    };
    let body = serde_json::to_vec(&request).map_err(|_| OpenError::Other)?;
    stream
        .set_write_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|_| OpenError::Other)?;
    let mut framed = (body.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&body);
    stream.write_all(&framed).map_err(|_| OpenError::Other)?;
    let mut conn = SocketConn {
        stream,
        buf: Vec::new(),
    };
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let bytes = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(OpenError::Other);
        }
        match conn.read_frame(remaining) {
            Ok(Some(bytes)) => break bytes,
            Ok(None) => return Err(OpenError::Other),
            Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
            Err(_) => return Err(OpenError::Other),
        }
    };
    if let Ok(reply) = serde_json::from_slice::<WatchReply>(&bytes) {
        if reply.request_id != request_id || reply.instance != instance.to_string() {
            return Err(OpenError::Other);
        }
        return Ok((reply, conn));
    }
    match serde_json::from_slice::<WireResponse>(&bytes) {
        Ok(response)
            if response
                .result
                .as_ref()
                .is_err_and(|error| error.code == ErrorCode::UnknownWireVersion) =>
        {
            Err(OpenError::Unsupported)
        }
        _ => Err(OpenError::Other),
    }
}

/// Everything one watch session needs besides the caller's output.
pub(crate) struct WatchSession<'a> {
    pub client: &'a dyn LocalClient,
    pub clock: &'a dyn Clock,
    pub socket: &'a Path,
    pub instance: uuid::Uuid,
    pub claim: CallerClaim,
    pub hint_path: Option<PathBuf>,
}

/// What the `mod.watch_v1` capability probe learned.
enum Probe {
    Supported,
    /// The daemon answered, and it has no such feature.
    Unsupported,
    /// The call itself failed (busy, timeout, booting daemon, transport loss): the daemon
    /// may well support the feature, so the mod retries instead of stopping for the session.
    Failed,
}

/// A fresh, fallible capability call: unlike `LocalClient::supports_capability`, a failed
/// call is not read as "no capabilities".
fn probe_mod_watch(client: &dyn LocalClient, budget: &CallBudget) -> Probe {
    match client.call_definitive(Command::Capabilities, budget) {
        Ok(Ok(CommandResult::Capabilities(list))) => {
            if list.capabilities.iter().any(|c| c == MOD_WATCH) {
                Probe::Supported
            } else {
                Probe::Unsupported
            }
        }
        // Correlated, definitive answers that mean "this daemon has no such feature".
        Ok(Err(error))
            if matches!(
                error.code,
                ErrorCode::UnknownWireVersion | ErrorCode::Unsupported
            ) =>
        {
            Probe::Unsupported
        }
        Ok(Ok(_)) => Probe::Unsupported,
        Ok(Err(_)) | Err(_) => Probe::Failed,
    }
}

/// Test entry: the bare marker prefix.
#[cfg(test)]
fn run_session(session: WatchSession<'_>, out: &mut dyn Write, alive: impl FnMut() -> bool) -> i32 {
    run_session_marked(session, Vec::new(), out, alive)
}

/// Capability check, registration, then the stream loop. Returns the exit code.
fn run_session_marked(
    session: WatchSession<'_>,
    command_prefix: Vec<String>,
    out: &mut dyn Write,
    mut alive: impl FnMut() -> bool,
) -> i32 {
    let mut state = EmitState::new(session.hint_path.clone()).with_command_prefix(command_prefix);
    match probe_mod_watch(session.client, &cooperative_budget(session.clock)) {
        Probe::Supported => {}
        Probe::Unsupported => return refuse(&mut state, out, WatchStatusReason::Unsupported),
        Probe::Failed => return refuse(&mut state, out, WatchStatusReason::DaemonUnavailable),
    }
    let seat = session.claim.seat.clone();
    let (reply, mut conn) = match open_watch(session.socket, session.instance, session.claim) {
        Ok(opened) => opened,
        Err(OpenError::Unavailable) => {
            return refuse(&mut state, out, WatchStatusReason::DaemonUnavailable);
        }
        Err(OpenError::Unsupported) => {
            return refuse(&mut state, out, WatchStatusReason::Unsupported);
        }
        Err(OpenError::Other) => return refuse(&mut state, out, WatchStatusReason::Error),
    };
    match reply.outcome {
        WatchOutcome::Refused(refusal) => {
            refuse(&mut state, out, WatchStatusReason::from(refusal.reason))
        }
        WatchOutcome::Accepted(accepted) => watch_loop(
            &mut conn,
            session.client,
            session.clock,
            &seat,
            accepted.attention_version,
            &mut state,
            out,
            &mut alive,
        ),
    }
}

/// Drain after `Accepted` and after every `Attention`; end on `Close`, stream
/// end, parent death or a closed stdout. Returns the exit code.
#[allow(clippy::too_many_arguments)]
fn watch_loop(
    conn: &mut dyn WatchConn,
    client: &dyn LocalClient,
    clock: &dyn Clock,
    seat: &SeatId,
    accepted_version: u64,
    state: &mut EmitState,
    out: &mut dyn Write,
    alive: &mut dyn FnMut() -> bool,
) -> i32 {
    macro_rules! out_or_quit {
        ($expr:expr) => {
            if $expr.is_err() {
                return 0;
            }
        };
    }
    let closing =
        |state: &mut EmitState, out: &mut dyn Write, reason: WatchStatusReason, exit: i32| {
            if emit_status(
                state,
                out,
                WatchStatus {
                    state: WatchStatusState::Closing,
                    reason: Some(reason),
                    exit: Some(exit),
                },
            )
            .is_err()
            {
                return 0;
            }
            exit
        };
    out_or_quit!(emit_status(
        state,
        out,
        WatchStatus {
            state: WatchStatusState::Connected,
            reason: None,
            exit: None,
        },
    ));
    let mut version = accepted_version;
    loop {
        match drain(client, clock, seat, version, state, out) {
            Ok(()) => {}
            Err(DrainError::Out) => return 0,
            Err(DrainError::Api | DrainError::Protocol) => {
                return closing(state, out, WatchStatusReason::Error, WATCH_EXIT_ERROR);
            }
        }
        // Block for the next Attention; every timeout is a liveness check.
        loop {
            if !alive() {
                return 0;
            }
            match conn.next_frame(STREAM_READ_TIMEOUT) {
                Ok(Some(WatchFrame::Attention { version: next })) => {
                    version = next;
                    break;
                }
                Ok(Some(WatchFrame::Close { reason })) => {
                    return closing(state, out, reason.into(), reason.exit_code());
                }
                Ok(None) => {
                    return closing(
                        state,
                        out,
                        WatchStatusReason::StreamEnded,
                        WATCH_EXIT_STREAM_ENDED,
                    );
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) => {}
                Err(_) => return closing(state, out, WatchStatusReason::Error, WATCH_EXIT_ERROR),
            }
        }
    }
}

/// Pages `InboxBatchV2` for the seat through the daemon.
struct ClientPageSource<'a> {
    client: &'a dyn LocalClient,
    clock: &'a dyn Clock,
    seat: &'a SeatId,
}

impl PageSource for ClientPageSource<'_> {
    fn page(&mut self, cursor: Option<String>) -> Result<Page<InboxBatchV2Item>, PageError> {
        let result = self
            .client
            .call(
                Command::InboxBatchV2(InboxQuery {
                    claim: None,
                    seat: Some(self.seat.clone()),
                    page: PageRequest {
                        cursor,
                        limit: PAGE_MAX_ITEMS as u16,
                        max_bytes: PAGE_MAX_BYTES as u32,
                    },
                }),
                &cooperative_budget(self.clock),
            )
            .map_err(|_: ApiError| PageError::Api)?;
        match result {
            CommandResult::InboxBatchV2(page) => Ok(page),
            _ => Err(PageError::Protocol),
        }
    }
}

/// Page `InboxBatchV2` for the seat until exhausted and print one line per
/// not-yet-emitted item. Invitations and warnings that wake the seat collapse
/// into one attention line per frame version; informational warnings raise
/// none.
pub(crate) fn drain(
    client: &dyn LocalClient,
    clock: &dyn Clock,
    seat: &SeatId,
    frame_version: u64,
    state: &mut EmitState,
    out: &mut dyn Write,
) -> Result<(), DrainError> {
    let mut source = ClientPageSource {
        client,
        clock,
        seat,
    };
    let attention_text =
        crate::protocol::watch::attention_marker_for(&state.assembler.command_prefix);
    drain_pages(
        &mut source,
        &mut state.assembler,
        frame_version,
        MAX_DRAIN_PAGES,
        |event| {
            let line = match event {
                DrainEvent::Item(item) => {
                    let id = item.id.as_str().to_owned();
                    WatchLine::new(
                        id,
                        match item.kind {
                            DeliveryKind::Lazy => WatchItem::Lazy(item.message),
                            DeliveryKind::Message => WatchItem::Message(item.message),
                        },
                    )
                }
                DrainEvent::Attention { version } => WatchLine::new(
                    format!("attention:{version}"),
                    WatchItem::Attention(WatchAttention {
                        attention_version: version,
                        text: attention_text.clone(),
                    }),
                ),
                DrainEvent::AttentionCleared { version } => WatchLine::new(
                    format!("attention_cleared:{version}"),
                    WatchItem::AttentionCleared(WatchAttentionCleared {
                        attention_version: version,
                    }),
                ),
                DrainEvent::Exhausted => return Ok(Stop::Continue),
            };
            write_line(out, &line).map(|()| Stop::Continue)
        },
    )
    .map_err(|error| match error {
        DrainPagesError::Page(PageError::Api) => DrainError::Api,
        DrainPagesError::Page(PageError::Protocol)
        | DrainPagesError::Assemble(_)
        | DrainPagesError::TooManyPages => DrainError::Protocol,
        DrainPagesError::Sink(_) => DrainError::Out,
    })
}

/// Hint only: remember that this seat was shown a cut body for `id`, so
/// `watch ack` can refuse it locally. The daemon decides truncation itself.
#[cfg(test)]
fn record_truncated(state: &EmitState, id: &str) {
    crate::delivery::assemble::record_truncated(state.assembler.truncation_hint.as_deref(), id);
}

/// `<instance dir>/contexts/watch-truncated-<seat>.ids`
fn hint_path(paths: &crate::daemon::paths::InstancePaths, seat: &SeatId) -> PathBuf {
    let name: String = seat
        .as_str()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    paths
        .instance_dir
        .join("contexts")
        .join(format!("watch-truncated-{name}.ids"))
}

/// Report mod deliveries (spec D6): one `ModAckItem` JSON line per id and exit
/// 0 iff every id has one. Hinted-truncated ids are refused locally; the rest
/// go to the daemon in calls of at most `MAX_BATCH_ITEMS` ids (the daemon
/// rejects a larger batch whole), each with its own operation id. A chunk
/// whose call fails makes only its own ids `retryable` (`busy` for a busy
/// daemon, else `unreachable`); when every chunk failed nothing is printed and
/// the exit is 1 (the mod then treats every id as retryable). Output order:
/// locally refused ids, then each chunk's lines in request order.
fn run_ack_lines(
    client: &dyn LocalClient,
    clock: &dyn Clock,
    request: &WatchAckRequest,
    claim: CallerClaim,
    hint: Option<&Path>,
    out: &mut dyn Write,
) -> i32 {
    let hinted = read_truncated_hint(hint);
    let mut requested: Vec<MessageId> = Vec::new();
    for id in &request.messages {
        if !requested.contains(id) {
            requested.push(id.clone());
        }
    }
    let (local, remote): (Vec<MessageId>, Vec<MessageId>) = requested
        .iter()
        .cloned()
        .partition(|id| hinted.contains(id.as_str()));
    let mut lines: Vec<ModAckItem> = local
        .into_iter()
        .map(|id| ModAckItem {
            id,
            result: ModAckOutcome::RefusedTerminal,
            reason: Some(ModAckReason::Truncated),
        })
        .collect();
    let mut failed_chunks = 0usize;
    let mut chunk_count = 0usize;
    for chunk in remote.chunks(crate::protocol::commands::MAX_BATCH_ITEMS) {
        chunk_count += 1;
        let result = client.call(
            Command::AckModDelivered(AckModDelivered {
                via: request.via,
                messages: chunk.to_vec(),
                operation: OperationId::new(uuid::Uuid::new_v4().to_string()),
                claim: claim.clone(),
            }),
            &cooperative_budget(clock),
        );
        match result {
            Ok(CommandResult::ModDeliveryAcked(report)) => {
                for id in chunk {
                    if let Some(item) = report.results.iter().find(|item| &item.id == id) {
                        lines.push(item.clone());
                    }
                }
            }
            other => {
                failed_chunks += 1;
                let reason = match other {
                    Err(error)
                        if matches!(error.code, ErrorCode::StoreBusy | ErrorCode::ServiceBusy) =>
                    {
                        ModAckReason::Busy
                    }
                    _ => ModAckReason::Unreachable,
                };
                lines.extend(chunk.iter().map(|id| ModAckItem {
                    id: id.clone(),
                    result: ModAckOutcome::Retryable,
                    reason: Some(reason),
                }));
            }
        }
    }
    if chunk_count > 0 && failed_chunks == chunk_count {
        return 1;
    }
    let complete = requested
        .iter()
        .all(|id| lines.iter().any(|line| &line.id == id));
    for line in &lines {
        let Ok(mut text) = serde_json::to_vec(line) else {
            return 1;
        };
        text.push(b'\n');
        if out.write_all(&text).and_then(|()| out.flush()).is_err() {
            return 0;
        }
    }
    if complete { 0 } else { 1 }
}

/// Why the caller could not be turned into a claim.
enum CallerFailure {
    Daemon,
    NoBinding,
    NoPushChannel,
    Other,
}

impl CallerFailure {
    fn reason(&self) -> WatchStatusReason {
        match self {
            Self::Daemon => WatchStatusReason::DaemonUnavailable,
            Self::NoBinding => WatchStatusReason::NoBinding,
            Self::NoPushChannel => WatchStatusReason::NO_PUSH_CHANNEL,
            Self::Other => WatchStatusReason::Error,
        }
    }
}

/// The watch child's caller. The mod passes the session it serves as `--session` (spec D3), so
/// the harness's session variable is not needed: an ancestor of a push-channel harness with the
/// variable unset is that session, fingerprinted by its own process.
fn watch_origin(
    parsed: &super::commands::ParsedCli,
    session: &str,
) -> Option<super::caller::CallerOrigin> {
    use super::caller::CallerOrigin;
    use crate::{
        harness::caller_env::{self, EnvClaim, EnvInputs, EnvRefusal},
        protocol::authority::{COOPERATIVE_ENV_SESSION_PROVENANCE, CallerRole},
    };
    let origin = super::caller::derive(parsed);
    let Some(CallerOrigin::Refused(EnvRefusal::MissingSession { harness, .. })) = &origin else {
        return origin;
    };
    let harness: &'static str = harness;
    if !crate::delivery::declares(
        harness,
        crate::harness::adapter::DeliveryMechanism::PushChannel,
    ) {
        return origin;
    }
    let Ok(agent) = crate::harness::registry::builtins().agent(harness) else {
        return origin;
    };
    let env = |name: &str| std::env::var(name).ok();
    let inputs = EnvInputs {
        env: &env,
        self_pid: std::process::id(),
        agent_id: None,
        processes: &crate::host::process_table::HostProcessTable,
    };
    match (
        caller_env::harness_fingerprint(&inputs, harness),
        NativeSessionId::parse(session.to_owned()),
    ) {
        (Ok(fingerprint), Ok(native_session)) => CallerOrigin::Agent(EnvClaim {
            harness: crate::protocol::authority::Harness::Agent(agent),
            native_session,
            agent_id: None,
            role: CallerRole::TopLevel,
            fingerprint,
            provenance: COOPERATIVE_ENV_SESSION_PROVENANCE,
        })
        .into(),
        _ => origin,
    }
}

/// Pane (or `--cooperative-*`) to seat to the seat's recorded harness context,
/// as a claim whose native session is the `--session` value (spec D3).
fn locate_claim<C, F>(
    parsed: &mut super::commands::ParsedCli,
    session: &str,
    caller_pane: Option<&str>,
    context: &crate::daemon::paths::RuntimeContext,
    paths: &crate::daemon::paths::InstancePaths,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
) -> Result<(CallerClaim, SeatId, uuid::Uuid), CallerFailure>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
{
    let (instance, client) = connection.get().map_err(|_| CallerFailure::Daemon)?;
    let origin = watch_origin(parsed, session);
    let _origin = super::caller::ActiveGuard::enter(origin.clone());
    let selection = match derive_caller(parsed, caller_pane, context, paths, connection, clock) {
        Ok(Some(selection)) => selection,
        Ok(None) => return Err(CallerFailure::NoBinding),
        Err(RunError::Api(error))
            if matches!(
                error.code,
                ErrorCode::InvalidRequest | ErrorCode::TargetUnresolved
            ) =>
        {
            return Err(CallerFailure::NoBinding);
        }
        Err(_) => return Err(CallerFailure::Other),
    };
    if let Some(super::caller::CallerOrigin::Agent(env)) = origin {
        // The mod's watch child runs under the session's harness: its claim is the environment's
        // (the fingerprint is the harness process's), with the `--session` the mod passed.
        if !crate::delivery::declares(
            env.harness.as_str(),
            crate::harness::adapter::DeliveryMechanism::PushChannel,
        ) {
            return Err(CallerFailure::NoPushChannel);
        }
        let generation = super::seat_generation(client, &parsed.output, &selection, clock.as_ref())
            .map_err(|_| CallerFailure::NoBinding)?;
        let current = super::caller::agent_context(
            &env,
            *instance,
            &selection.seat,
            &selection.target,
            generation,
        );
        let claim = bridge::caller_claim(&current).map_err(|_| CallerFailure::NoBinding)?;
        let claim = own_session_claim(claim, session, Some(env.fingerprint))?;
        return Ok((claim, selection.seat, *instance));
    }
    let contexts =
        seat_contexts(paths, *instance, &selection.seat).map_err(|_| CallerFailure::Other)?;
    let current = contexts
        .current()
        .map_err(|_| CallerFailure::Other)?
        .ok_or(CallerFailure::NoBinding)?;
    if !crate::delivery::declares(
        current.harness.occupant().as_str(),
        crate::harness::adapter::DeliveryMechanism::PushChannel,
    ) {
        return Err(CallerFailure::NoPushChannel);
    }
    let claim = bridge::caller_claim(&current).map_err(|_| CallerFailure::NoBinding)?;
    let claim = own_session_claim(claim, session, None)?;
    Ok((claim, selection.seat, *instance))
}

/// The watch child's claim names the binding of the session it serves (spec §3 Wake): the
/// top-level Claude binding keyed by the seat and the `--session` the mod passed. A seat holds a
/// binding per session, so the registration must carry this session and never an agent id; the
/// daemon resolves it to that binding and holds the channel there.
fn own_session_claim(
    mut claim: CallerClaim,
    session: &str,
    liveness: Option<crate::protocol::authority::LivenessFingerprint>,
) -> Result<CallerClaim, CallerFailure> {
    claim.native_session =
        NativeSessionId::parse(session.to_owned()).map_err(|_| CallerFailure::Other)?;
    claim.role = crate::protocol::authority::CallerRole::TopLevel;
    claim.agent_id = None;
    if liveness.is_some() {
        claim.liveness = liveness;
    }
    Ok(claim)
}

#[allow(clippy::too_many_arguments)]
/// `watch` after the pre-checks: locate the caller, register, stream.
pub(super) fn run_watch<C, F, W>(
    parsed: &mut super::commands::ParsedCli,
    request: &WatchRequest,
    caller_pane: Option<&str>,
    context: &crate::daemon::paths::RuntimeContext,
    paths: &crate::daemon::paths::InstancePaths,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
    writer: &mut W,
) -> Result<(), RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
    W: Write,
{
    let mut state = EmitState::default();
    let (claim, seat, instance) = match locate_claim(
        parsed,
        &request.session,
        caller_pane,
        context,
        paths,
        connection,
        clock,
    ) {
        Ok(found) => found,
        Err(failure) => return exit_result(refuse(&mut state, writer, failure.reason())),
    };
    let Ok((_, client)) = connection.get() else {
        return exit_result(refuse(
            &mut state,
            writer,
            WatchStatusReason::DaemonUnavailable,
        ));
    };
    let socket = match published_endpoint(paths) {
        Ok((_, descriptor)) => descriptor.endpoint,
        Err(_) => {
            return exit_result(refuse(
                &mut state,
                writer,
                WatchStatusReason::DaemonUnavailable,
            ));
        }
    };
    let start_ppid = system_ppid();
    // What a bare `herdr-threads` in the session's shell (this process's
    // environment, as for the hooks) would reach decides the marker's selectors.
    let command_prefix = super::hook::cli_prefix(&super::hook::pane_selectors(
        Some(&context.state_dir),
        Some(&context.host_endpoint),
        &super::hook::pane_inputs(),
    ));
    let code = run_session_marked(
        WatchSession {
            client,
            clock: clock.as_ref(),
            socket: &socket,
            instance,
            claim,
            hint_path: Some(hint_path(paths, &seat)),
        },
        command_prefix,
        writer,
        || parent_alive(start_ppid, system_ppid),
    );
    exit_result(code)
}

#[allow(clippy::too_many_arguments)]
/// `watch ack` after the pre-check.
pub(super) fn run_ack<C, F, W>(
    parsed: &mut super::commands::ParsedCli,
    request: &WatchAckRequest,
    caller_pane: Option<&str>,
    context: &crate::daemon::paths::RuntimeContext,
    paths: &crate::daemon::paths::InstancePaths,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
    writer: &mut W,
) -> Result<(), RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
    W: Write,
{
    let Ok((claim, seat, _)) = locate_claim(
        parsed,
        &request.session,
        caller_pane,
        context,
        paths,
        connection,
        clock,
    ) else {
        return Err(RunError::Exit(1));
    };
    let Ok((_, client)) = connection.get() else {
        return Err(RunError::Exit(1));
    };
    let hint = hint_path(paths, &seat);
    exit_result(run_ack_lines(
        client,
        clock.as_ref(),
        request,
        claim,
        Some(&hint),
        writer,
    ))
}

#[cfg(test)]
use crate::protocol::watch::{WatchMessage, truncation_marker};
#[cfg(test)]
use std::collections::HashSet;

#[cfg(test)]
#[path = "../../tests/cli/watch.rs"]
mod tests;
