//! Hidden `watch` and `watch ack` subcommands: the Claude mod's delivery child.
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
    harness::{bridge, context::Harness},
    ports::LocalClient,
    protocol::{
        authority::CallerClaim,
        capabilities::MOD_WATCH,
        commands::{AckModDelivered, Command, InboxQuery},
        ids::{MessageId, NativeSessionId, OperationId, SeatId},
        pagination::PageRequest,
        results::{ApiError, CommandResult, ErrorCode, InboxBatchV2Item},
        time::Clock,
        watch::{
            ModAckItem, ModAckOutcome, ModAckReason, ModDeliveryVia, WATCH_BODY_LIMIT_BYTES,
            WATCH_EXIT_ERROR, WATCH_EXIT_STREAM_ENDED, WATCH_PAGE_MAX_BYTES, WATCH_PAGE_MAX_ITEMS,
            WatchAttention, WatchFrame, WatchItem, WatchLine, WatchMessage, WatchOutcome,
            WatchReply, WatchRequest as WireWatch, WatchStatus, WatchStatusReason,
            WatchStatusState, WatchWireRequest, truncation_marker,
        },
        wire::{PROTOCOL_VERSION, WireResponse},
    },
};
use std::{
    collections::{HashMap, HashSet},
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
/// Newest ids kept in the truncation hint file.
const HINT_MAX_LINES: usize = 1024;
/// Upper bound on pages followed in one drain (a cursor loop guard).
const MAX_DRAIN_PAGES: usize = 4096;

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

/// Process-wide emit bookkeeping: the status counter, ids already streamed by
/// this process, and the truncation hint file.
#[derive(Debug, Default)]
pub(crate) struct EmitState {
    next_status: u64,
    emitted: HashSet<String>,
    hint_path: Option<PathBuf>,
}

impl EmitState {
    fn new(hint_path: Option<PathBuf>) -> Self {
        Self {
            hint_path,
            ..Self::default()
        }
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

/// Capability check, registration, then the stream loop. Returns the exit code.
fn run_session(
    session: WatchSession<'_>,
    out: &mut dyn Write,
    mut alive: impl FnMut() -> bool,
) -> i32 {
    let mut state = EmitState::new(session.hint_path.clone());
    if !session
        .client
        .supports_capability(MOD_WATCH, &cooperative_budget(session.clock))
    {
        return refuse(&mut state, out, WatchStatusReason::Unsupported);
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

/// Text assembled so far for one multi-chunk body.
struct Partial {
    body: String,
}

/// Page `InboxBatchV2` for the seat until exhausted and print one line per
/// not-yet-emitted item. Invitations and warnings collapse into one attention
/// line per frame version.
pub(crate) fn drain(
    client: &dyn LocalClient,
    clock: &dyn Clock,
    seat: &SeatId,
    frame_version: u64,
    state: &mut EmitState,
    out: &mut dyn Write,
) -> Result<(), DrainError> {
    let mut cursor: Option<String> = None;
    let mut partial: HashMap<String, Partial> = HashMap::new();
    let mut saw_attention = false;
    for _ in 0..MAX_DRAIN_PAGES {
        let result = client.call(
            Command::InboxBatchV2(InboxQuery {
                seat: Some(seat.clone()),
                page: PageRequest {
                    cursor: cursor.take(),
                    limit: WATCH_PAGE_MAX_ITEMS as u16,
                    max_bytes: WATCH_PAGE_MAX_BYTES as u32,
                },
            }),
            &cooperative_budget(clock),
        )?;
        let CommandResult::InboxBatchV2(page) = result else {
            return Err(DrainError::Protocol);
        };
        for item in page.items {
            match item {
                InboxBatchV2Item::Invitation { .. } | InboxBatchV2Item::Warning { .. } => {
                    saw_attention = true;
                }
                InboxBatchV2Item::Message {
                    thread,
                    message,
                    sender,
                    author_role,
                    relays_user,
                    user_intent,
                    body,
                    body_start,
                    body_end,
                    body_len,
                    ack_candidate,
                    ..
                } => emit_chunk(
                    state,
                    out,
                    &mut partial,
                    Chunk {
                        lazy: false,
                        ack_required: ack_candidate.is_some(),
                        thread,
                        message,
                        sender,
                        author_role,
                        relays_user,
                        user_intent,
                        body,
                        body_start,
                        body_end,
                        body_len,
                    },
                )?,
                InboxBatchV2Item::LazyMessage {
                    thread,
                    message,
                    sender,
                    author_role,
                    relays_user,
                    user_intent,
                    body,
                    body_start,
                    body_end,
                    body_len,
                    ..
                } => emit_chunk(
                    state,
                    out,
                    &mut partial,
                    Chunk {
                        lazy: true,
                        ack_required: false,
                        thread,
                        message,
                        sender,
                        author_role,
                        relays_user,
                        user_intent,
                        body,
                        body_start,
                        body_end,
                        body_len,
                    },
                )?,
            }
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => {
                if saw_attention {
                    let id = format!("attention:{frame_version}");
                    if state.emitted.insert(id.clone()) {
                        write_line(
                            out,
                            &WatchLine::new(
                                id,
                                WatchItem::Attention(WatchAttention {
                                    attention_version: frame_version,
                                    text: crate::notification::policy::MARKER.to_owned(),
                                }),
                            ),
                        )?;
                    }
                }
                return Ok(());
            }
        }
    }
    Err(DrainError::Protocol)
}

struct Chunk {
    lazy: bool,
    ack_required: bool,
    thread: crate::protocol::ids::ThreadId,
    message: MessageId,
    sender: Option<SeatId>,
    author_role: Option<crate::protocol::summary::AuthorRole>,
    relays_user: bool,
    user_intent: Option<crate::protocol::summary::UserIntent>,
    body: String,
    body_start: u64,
    body_end: u64,
    body_len: u64,
}

/// Add one body chunk; print the line once the first
/// `WATCH_BODY_LIMIT_BYTES` bytes or the whole body are assembled.
fn emit_chunk(
    state: &mut EmitState,
    out: &mut dyn Write,
    partial: &mut HashMap<String, Partial>,
    chunk: Chunk,
) -> Result<(), DrainError> {
    let id = chunk.message.as_str().to_owned();
    if state.emitted.contains(&id) {
        return Ok(());
    }
    let entry = partial.entry(id.clone()).or_insert_with(|| Partial {
        body: String::new(),
    });
    if chunk.body_start == 0 {
        entry.body.clear();
    }
    if chunk.body_start != entry.body.len() as u64 {
        partial.remove(&id);
        return Err(DrainError::Protocol);
    }
    entry.body.push_str(&chunk.body);
    let whole = chunk.body_end >= chunk.body_len;
    if !whole && entry.body.len() < WATCH_BODY_LIMIT_BYTES {
        return Ok(());
    }
    let assembled = partial.remove(&id).map(|p| p.body).unwrap_or_default();
    let truncated = chunk.body_len > WATCH_BODY_LIMIT_BYTES as u64;
    let body = if truncated {
        let mut cut = WATCH_BODY_LIMIT_BYTES.min(assembled.len());
        while !assembled.is_char_boundary(cut) {
            cut -= 1;
        }
        record_truncated(state, &id);
        format!("{}{}", &assembled[..cut], truncation_marker(&chunk.message))
    } else {
        assembled
    };
    let message = WatchMessage {
        thread: chunk.thread,
        thread_name: None,
        sender: chunk.sender,
        sender_name: None,
        author_role: chunk.author_role,
        relays_user: chunk.relays_user,
        user_intent: chunk.user_intent,
        body,
        body_len: chunk.body_len,
        truncated,
        ack_required: chunk.ack_required,
    };
    let item = if chunk.lazy {
        WatchItem::Lazy(message)
    } else {
        WatchItem::Message(message)
    };
    state.emitted.insert(id.clone());
    write_line(out, &WatchLine::new(id, item))?;
    Ok(())
}

/// Hint only: remember that this seat was shown a cut body for `id`, so
/// `watch ack` can refuse it locally. The daemon decides truncation itself.
fn record_truncated(state: &EmitState, id: &str) {
    let Some(path) = &state.hint_path else {
        return;
    };
    let mut lines: Vec<String> = std::fs::read_to_string(path)
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    if lines.iter().any(|line| line == id) {
        return;
    }
    lines.push(id.to_owned());
    if lines.len() > HINT_MAX_LINES {
        lines.drain(..lines.len() - HINT_MAX_LINES);
    }
    let mut text = lines.join("\n");
    text.push('\n');
    let temp = path.with_extension("ids.tmp");
    if std::fs::write(&temp, text).is_ok() {
        let _ = std::fs::rename(&temp, path);
    }
}

fn read_truncated_hint(path: Option<&Path>) -> HashSet<String> {
    path.and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default()
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
/// 0 iff every id has one. Any error prints nothing and exits 1 (the mod then
/// treats every id as retryable). Hinted-truncated ids are refused locally.
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
    if !remote.is_empty() {
        let result = client.call(
            Command::AckModDelivered(AckModDelivered {
                via: request.via,
                messages: remote.clone(),
                operation: OperationId::new(uuid::Uuid::new_v4().to_string()),
                claim,
            }),
            &cooperative_budget(clock),
        );
        let Ok(CommandResult::ModDeliveryAcked(report)) = result else {
            return 1;
        };
        for id in &remote {
            if let Some(item) = report.results.iter().find(|item| &item.id == id) {
                lines.push(item.clone());
            }
        }
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
    NotClaude,
    Other,
}

impl CallerFailure {
    fn reason(&self) -> WatchStatusReason {
        match self {
            Self::Daemon => WatchStatusReason::DaemonUnavailable,
            Self::NoBinding => WatchStatusReason::NoBinding,
            Self::NotClaude => WatchStatusReason::NotClaude,
            Self::Other => WatchStatusReason::Error,
        }
    }
}

/// Pane (or `--cooperative-*`) to seat to the seat's recorded Claude context,
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
    let (instance, _) = connection.get().map_err(|_| CallerFailure::Daemon)?;
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
    let contexts =
        seat_contexts(paths, *instance, &selection.seat).map_err(|_| CallerFailure::Other)?;
    let current = contexts
        .current()
        .map_err(|_| CallerFailure::Other)?
        .ok_or(CallerFailure::NoBinding)?;
    if current.harness != Harness::Claude {
        return Err(CallerFailure::NotClaude);
    }
    let mut claim = bridge::caller_claim(&current).map_err(|_| CallerFailure::NoBinding)?;
    claim.native_session =
        NativeSessionId::parse(session.to_owned()).map_err(|_| CallerFailure::Other)?;
    Ok((claim, selection.seat, *instance))
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
    let code = run_session(
        WatchSession {
            client,
            clock: clock.as_ref(),
            socket: &socket,
            instance,
            claim,
            hint_path: Some(hint_path(paths, &seat)),
        },
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
#[path = "../../tests/cli/watch.rs"]
mod tests;
