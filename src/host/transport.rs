//! One bounded audited Herdr API exchange. Dropping this future closes its socket.

use super::continuity::{
    KernelProcessInfo, LocalEndpointWitness, ProcessInfoProvider, capture_peer_witness_with,
    recheck_witness, socket_identity,
};
use crate::protocol::{
    results::{ApiError, ErrorCode},
    time::{CallBudget, Clock},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    cell::Cell,
    future::Future,
    io,
    path::Path,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

const MAX_REQUEST: usize = 1024 * 1024;
const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(5);

pub(crate) type StartValidator<'a> =
    dyn Fn(&Value, &LocalEndpointWitness) -> Result<(), ApiError> + Sync + 'a;

struct OperationFence<'a> {
    pong: &'a Value,
    expected: &'a LocalEndpointWitness,
    validate: &'a StartValidator<'a>,
    attempted: &'a Cell<bool>,
}

const CONFIRMED_START_REFUSALS: [&str; 3] = [
    "agent_pane_busy",
    "agent_name_taken",
    "agent_process_hint_unsupported",
];

fn compatible_pong(ping: &Value) -> Result<&Value, ApiError> {
    if let Some(error) = super::observation::structured_host_error(ping) {
        return Err(error);
    }
    let pong = ping
        .get("result")
        .ok_or_else(|| error(ErrorCode::Unsupported, "host API ping failed"))?;
    if pong.get("type").and_then(Value::as_str) != Some("pong")
        || !super::compatibility::supports_json_api(
            pong.get("version").and_then(Value::as_str),
            pong.get("protocol").and_then(Value::as_u64),
        )
    {
        return Err(error(
            ErrorCode::Unsupported,
            "host API version or protocol mismatch",
        ));
    }
    Ok(pong)
}

#[derive(Deserialize)]
struct Envelope {
    id: String,
    #[serde(default, deserialize_with = "branch")]
    result: Option<Value>,
    #[serde(default, deserialize_with = "branch")]
    error: Option<Value>,
}

fn branch<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(decoder).map(Some)
}

struct RequestBuffer(Vec<u8>);
impl io::Write for RequestBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > (MAX_REQUEST - 1).saturating_sub(self.0.len()) {
            return Err(io::Error::other("host API request exceeds byte limit"));
        }
        let needed = self.0.len() + bytes.len();
        if needed > self.0.capacity() {
            let capacity = self
                .0
                .capacity()
                .saturating_mul(2)
                .max(needed)
                .min(MAX_REQUEST);
            self.0.reserve_exact(capacity - self.0.len());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_request(value: &Value) -> Result<Vec<u8>, ApiError> {
    let mut buffer = RequestBuffer(Vec::new());
    serde_json::to_writer(&mut buffer, value).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "host API request encoding exceeds byte limit or failed",
        )
    })?;
    buffer.0.push(b'\n');
    Ok(buffer.0)
}

fn error(code: ErrorCode, detail: impl Into<String>) -> ApiError {
    ApiError::new(code, detail)
}

fn check(
    clock: &dyn Clock,
    budget: &CallBudget,
    started: Instant,
    limit: Duration,
) -> Result<(), ApiError> {
    if budget.cancellation.is_cancelled() {
        return Err(error(ErrorCode::Cancelled, "host API call cancelled"));
    }
    if clock.monotonic_now() >= budget.deadline || started.elapsed() >= limit {
        return Err(error(
            ErrorCode::DeadlineExceeded,
            "host API call timed out; outcome unknown",
        ));
    }
    Ok(())
}

async fn bounded<F, T>(
    future: F,
    clock: &dyn Clock,
    budget: &CallBudget,
    started: Instant,
    limit: Duration,
) -> Result<T, ApiError>
where
    F: Future<Output = io::Result<T>>,
{
    tokio::pin!(future);
    loop {
        check(clock, budget, started, limit)?;
        tokio::select! {
            result = &mut future => {
                check(clock, budget, started, limit)?;
                return result.map_err(|e| error(ErrorCode::HostUnavailable, format!("host API transport: {e}")));
            }
            _ = tokio::time::sleep(POLL) => {}
        }
    }
}

fn budgeted_capture<T>(
    operation: impl FnOnce() -> Result<T, super::continuity::CaptureError>,
    clock: &dyn Clock,
    budget: &CallBudget,
    started: Instant,
    limit: Duration,
) -> Result<T, ApiError> {
    let result = operation();
    check(clock, budget, started, limit)?;
    result.map_err(witness_error)
}

// Allowed: one host RPC exchange with its budget, deadline and peer-identity provider.
#[allow(clippy::too_many_arguments)]
async fn exchange(
    socket: &Path,
    id: &str,
    method: &str,
    params: Value,
    clock: &dyn Clock,
    budget: &CallBudget,
    started: Instant,
    limit: Duration,
    provider: Option<&dyn ProcessInfoProvider>,
    fence: Option<&OperationFence<'_>>,
) -> Result<(Value, Option<LocalEndpointWitness>), ApiError> {
    check(clock, budget, started, limit)?;
    let request = encode_request(&json!({"id":id,"method":method,"params":params}))?;
    check(clock, budget, started, limit)?;
    let path_before = budgeted_capture(
        || provider.map(|_| socket_identity(socket)).transpose(),
        clock,
        budget,
        started,
        limit,
    )?;
    let connect = async {
        UnixStream::connect(socket).await.map_err(|e| {
            let (_, reason) = crate::client::classify_connect_error(&e);
            io::Error::new(e.kind(), format!("{reason}: {e}"))
        })
    };
    let mut stream = bounded(connect, clock, budget, started, limit).await?;
    let witness = budgeted_capture(
        || {
            provider
                .map(|provider| capture_peer_witness_with(&stream, socket, provider))
                .transpose()
        },
        clock,
        budget,
        started,
        limit,
    )?;
    if path_before.as_ref() != witness.as_ref().map(|w| &w.socket) {
        return Err(error(
            ErrorCode::StaleHostObservation,
            "host endpoint changed during connect",
        ));
    }
    if let Some(fence) = fence {
        if witness.as_ref() != Some(fence.expected) {
            return Err(error(
                ErrorCode::StaleHostObservation,
                "host endpoint changed before operation write",
            ));
        }
        (fence.validate)(fence.pong, fence.expected)?;
        check(clock, budget, started, limit)?;
    }
    let mut position = 0;
    while position < request.len() {
        let write = stream.write(&request[position..]);
        tokio::pin!(write);
        let attempt = std::future::poll_fn(|cx| {
            if let Some(fence) = fence {
                fence.attempted.set(true);
            }
            write.as_mut().poll(cx)
        });
        let count = bounded(attempt, clock, budget, started, limit).await?;
        if count == 0 {
            return Err(error(ErrorCode::HostUnavailable, "host API write closed"));
        }
        position += count;
    }
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    let mut complete = false;
    loop {
        let count = bounded(stream.read(&mut chunk), clock, budget, started, limit).await?;
        if count == 0 {
            if complete {
                break;
            }
            return Err(error(
                ErrorCode::HostUnavailable,
                "host API response ended before LF",
            ));
        }
        if complete {
            return Err(error(
                ErrorCode::StaleHostObservation,
                "host API sent more than one response frame",
            ));
        }
        if let Some(end) = chunk[..count].iter().position(|byte| *byte == b'\n') {
            if response.len() + end + 1 > MAX_RESPONSE {
                return Err(error(
                    ErrorCode::StaleHostObservation,
                    "host API response exceeds byte limit",
                ));
            }
            if end + 1 != count {
                return Err(error(
                    ErrorCode::StaleHostObservation,
                    "host API sent more than one response frame",
                ));
            }
            response.extend_from_slice(&chunk[..=end]);
            // The pinned ordinary server owns this stream by value and returns
            // after writing (api/server.rs:102-112,156-163,280-302). Await EOF
            // within the same budget to reject delayed extra frames.
            complete = true;
            continue;
        }
        if response.len() + count > MAX_RESPONSE {
            return Err(error(
                ErrorCode::StaleHostObservation,
                "host API response exceeds byte limit",
            ));
        }
        response.extend_from_slice(&chunk[..count]);
    }
    check(clock, budget, started, limit)?;
    if let Some(provider) = provider {
        let before = witness.as_ref().ok_or_else(|| {
            error(
                ErrorCode::StaleHostObservation,
                "endpoint witness unavailable",
            )
        })?;
        let after = budgeted_capture(
            || recheck_witness(before, provider),
            clock,
            budget,
            started,
            limit,
        )?;
        if witness.as_ref() != Some(&after) {
            return Err(error(
                ErrorCode::StaleHostObservation,
                "host endpoint witness changed during response",
            ));
        }
    }
    let envelope: Envelope = serde_json::from_slice(&response).map_err(|_| {
        error(
            ErrorCode::StaleHostObservation,
            "malformed host API response",
        )
    })?;
    if envelope.id != id
        || envelope.result.is_some() == envelope.error.is_some()
        || !envelope
            .result
            .as_ref()
            .or(envelope.error.as_ref())
            .is_some_and(Value::is_object)
    {
        return Err(error(
            ErrorCode::StaleHostObservation,
            "host API response correlation failed",
        ));
    }
    if let Some(body) = &envelope.error
        && (body
            .get("code")
            .and_then(Value::as_str)
            .is_none_or(|code| code.is_empty())
            || body.get("message").and_then(Value::as_str).is_none())
    {
        return Err(error(
            ErrorCode::StaleHostObservation,
            "malformed host API error",
        ));
    }
    let value = match envelope.result {
        Some(result) => json!({"id":envelope.id,"result":result}),
        None => json!({"id":envelope.id,"error":envelope.error}),
    };
    check(clock, budget, started, limit)?;
    Ok((value, witness))
}

// Allowed: exchange's inputs for the blocking path.
#[allow(clippy::too_many_arguments)]
fn request_inner(
    socket: &Path,
    id: &str,
    method: &str,
    params: Value,
    clock: &dyn Clock,
    budget: &CallBudget,
    limit: Duration,
    provider: Option<&dyn ProcessInfoProvider>,
) -> Result<(String, Option<LocalEndpointWitness>), ApiError> {
    request_inner_response(
        socket, id, method, params, clock, budget, limit, provider, false,
    )
}

#[allow(clippy::too_many_arguments)]
fn request_inner_response(
    socket: &Path,
    id: &str,
    method: &str,
    params: Value,
    clock: &dyn Clock,
    budget: &CallBudget,
    limit: Duration,
    provider: Option<&dyn ProcessInfoProvider>,
    preserve_start_refusal: bool,
) -> Result<(String, Option<LocalEndpointWitness>), ApiError> {
    let started = Instant::now();
    // HostPort is synchronous and may be called from inside a Tokio runtime.
    // Keep this one transport task owned and joined on every return path.
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                    .map_err(|e| {
                        error(ErrorCode::HostUnavailable, format!("host API runtime: {e}"))
                    })?;
                runtime.block_on(async {
                    let (ping, ping_witness) = exchange(
                        socket,
                        &format!("{id}:ping"),
                        "ping",
                        json!({}),
                        clock,
                        budget,
                        started,
                        limit,
                        provider,
                        None,
                    )
                    .await?;
                    compatible_pong(&ping)?;
                    let (result, witness) = exchange(
                        socket, id, method, params, clock, budget, started, limit, provider, None,
                    )
                    .await?;
                    if ping_witness != witness {
                        return Err(error(
                            ErrorCode::StaleHostObservation,
                            "host endpoint changed between ping and operation",
                        ));
                    }
                    let confirmed_refusal = preserve_start_refusal
                        && result
                            .pointer("/error/code")
                            .and_then(Value::as_str)
                            .is_some_and(|code| CONFIRMED_START_REFUSALS.contains(&code));
                    if !confirmed_refusal
                        && let Some(error) = super::observation::structured_host_error(&result)
                    {
                        return Err(error);
                    }
                    let expected = match method {
                        "pane.get" => Some("pane_info"),
                        "pane.current" => Some("pane_current"),
                        "pane.read" => Some("pane_read"),
                        "session.snapshot" => Some("session_snapshot"),
                        "agent.prompt" => Some("agent_prompted"),
                        "agent.start" => Some("agent_started"),
                        "agent.get" => Some("agent_info"),
                        "agent.read" => Some("pane_read"),
                        // Composer input sends carry no data the adapter
                        // uses: a structured error was refused above, and a
                        // mutation that already happened must not be
                        // reported as failed over an unmodelled result type.
                        "pane.send_keys" | "pane.send_text" => None,
                        _ => {
                            return Err(error(
                                ErrorCode::InvalidRequest,
                                "unsupported host API method",
                            ));
                        }
                    };
                    if !confirmed_refusal
                        && expected.is_some_and(|expected| {
                            result.pointer("/result/type").and_then(Value::as_str) != Some(expected)
                        })
                    {
                        return Err(error(
                            ErrorCode::StaleHostObservation,
                            "unexpected host API result type",
                        ));
                    }
                    let encoded = serde_json::to_string(&result).map_err(|_| {
                        error(
                            ErrorCode::StaleHostObservation,
                            "host API response encoding failed",
                        )
                    })?;
                    check(clock, budget, started, limit)?;
                    Ok((encoded, witness))
                })
            })
            .join()
            .map_err(|_| error(ErrorCode::HostUnavailable, "host API task panicked"))?
    })
}

/// Keep exact confirmed start refusals available to the existing unhinted
/// run/dispatch path. Ordinary request wrappers still map errors as before.
pub(crate) fn request_unhinted_start(
    socket: &Path,
    id: &str,
    params: Value,
    clock: &dyn Clock,
    budget: &CallBudget,
    limit: Duration,
) -> Result<String, ApiError> {
    request_inner_response(
        socket,
        id,
        "agent.start",
        params,
        clock,
        budget,
        limit,
        None,
        true,
    )
    .map(|(body, _)| body)
}

#[allow(clippy::too_many_arguments)]
fn guarded_start_inner(
    socket: &Path,
    id: &str,
    params: Value,
    clock: &dyn Clock,
    budget: &CallBudget,
    limit: Duration,
    provider: &dyn ProcessInfoProvider,
    validate: &StartValidator<'_>,
) -> Result<WitnessedResponse, crate::ports::NativeLaunchFailure> {
    use crate::ports::{NativeLaunchFailure, NativeSubmission};
    let started = Instant::now();
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let attempted = Cell::new(false);
                let result: Result<WitnessedResponse, ApiError> = (|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_io()
                        .enable_time()
                        .build()
                        .map_err(|e| {
                            error(ErrorCode::HostUnavailable, format!("host API runtime: {e}"))
                        })?;
                    runtime.block_on(async {
                        let (ping, witness) = exchange(
                            socket,
                            &format!("{id}:ping"),
                            "ping",
                            json!({}),
                            clock,
                            budget,
                            started,
                            limit,
                            Some(provider),
                            None,
                        )
                        .await?;
                        let pong = compatible_pong(&ping)?;
                        if pong
                            .pointer("/capabilities/agent_start_process_hint_v1")
                            .and_then(Value::as_bool)
                            != Some(true)
                        {
                            return Err(error(
                                ErrorCode::Unsupported,
                                "host does not advertise agent_start_process_hint_v1",
                            ));
                        }
                        let witness = witness.ok_or_else(|| {
                            error(
                                ErrorCode::StaleHostObservation,
                                "endpoint witness unavailable",
                            )
                        })?;
                        validate(pong, &witness)?;
                        let fence = OperationFence {
                            pong,
                            expected: &witness,
                            validate,
                            attempted: &attempted,
                        };
                        let (result, _) = exchange(
                            socket,
                            id,
                            "agent.start",
                            params,
                            clock,
                            budget,
                            started,
                            limit,
                            Some(provider),
                            Some(&fence),
                        )
                        .await?;
                        let code = result.pointer("/error/code").and_then(Value::as_str);
                        if let Some(host_error) = super::observation::structured_host_error(&result)
                        {
                            if code.is_some_and(|code| CONFIRMED_START_REFUSALS.contains(&code)) {
                                attempted.set(false);
                                if code == Some("agent_process_hint_unsupported") {
                                    return Err(host_error);
                                }
                            } else {
                                return Err(host_error);
                            }
                        } else if result.pointer("/result/type").and_then(Value::as_str)
                            != Some("agent_started")
                        {
                            return Err(error(
                                ErrorCode::StaleHostObservation,
                                "unexpected host API result type",
                            ));
                        }
                        check(clock, budget, started, limit)?;
                        let body = serde_json::to_string(&result).map_err(|_| {
                            error(
                                ErrorCode::StaleHostObservation,
                                "host API response encoding failed",
                            )
                        })?;
                        Ok(WitnessedResponse { body, witness })
                    })
                })();
                result.map_err(|error| NativeLaunchFailure {
                    error,
                    submission: if attempted.get() {
                        NativeSubmission::Possible
                    } else {
                        NativeSubmission::NotSubmitted
                    },
                })
            })
            .join()
            .map_err(|_| NativeLaunchFailure {
                error: error(
                    ErrorCode::HostUnavailable,
                    "host API task panicked; outcome unknown",
                ),
                submission: NativeSubmission::Possible,
            })?
    })
}

/// Required-mode start: each actual ping negotiates capability, and the
/// connected operation peer is fenced before polling the first write.
pub(crate) fn request_guarded_start(
    socket: &Path,
    id: &str,
    params: Value,
    clock: &dyn Clock,
    budget: &CallBudget,
    limit: Duration,
    validate: &StartValidator<'_>,
) -> Result<WitnessedResponse, crate::ports::NativeLaunchFailure> {
    guarded_start_inner(
        socket,
        id,
        params,
        clock,
        budget,
        limit,
        &KernelProcessInfo,
        validate,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_is_bounded_before_buffer_growth_and_counts_lf() {
        let request = json!({"text":"\u{0}".repeat(200_000)});
        assert_eq!(
            encode_request(&request).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        let request = json!({"text":"x".repeat(MAX_REQUEST - 12)});
        assert_eq!(encode_request(&request).unwrap().len(), MAX_REQUEST);
        let request = json!({"text":"x".repeat(MAX_REQUEST - 11)});
        assert_eq!(
            encode_request(&request).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
}

#[derive(Debug, Clone)]
pub struct WitnessedResponse {
    pub body: String,
    pub witness: super::continuity::LocalEndpointWitness,
}

#[cfg(all(test, target_os = "macos"))]
mod process_hint_tests {
    use super::*;
    use crate::{
        host::native::tests::HintSocketFixture,
        ports::NativeSubmission,
        protocol::time::{Cancellation, MonoInstant, UtcMillis},
    };
    use std::{
        io::Write,
        sync::{
            Arc,
            atomic::{AtomicU64, AtomicUsize, Ordering},
        },
    };

    struct HintClock(Arc<AtomicU64>);
    impl Clock for HintClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(Ordering::Acquire))
        }
    }
    struct ChangingProvider {
        calls: AtomicUsize,
        change_at: usize,
        peer: bool,
    }
    impl ProcessInfoProvider for ChangingProvider {
        fn process_info(
            &self,
            pid: u32,
        ) -> Result<super::super::continuity::ProcessInfo, super::super::continuity::CaptureError>
        {
            let mut info = KernelProcessInfo.process_info(pid)?;
            if self.calls.fetch_add(1, Ordering::AcqRel) >= self.change_at {
                if self.peer {
                    info.pid += 1;
                } else {
                    info.start_seconds += 1;
                }
            }
            Ok(info)
        }
    }
    fn pong() -> Value {
        json!({"type":"pong","version":"0.9.1","protocol":22,"capabilities":{"agent_start_process_hint_v1":true}})
    }
    fn params() -> Value {
        json!({"name":"hint-worker","kind":"codex","pane_id":"w4:p1","args":["space arg","apostrophe's arg"],"timeout_ms":2000,"process_hint":true})
    }
    fn fixture(pong: Value) -> HintSocketFixture {
        HintSocketFixture::new(move |stream, wire| {
            let result = if wire["method"] == "ping" {
                pong.clone()
            } else {
                json!({"type":"agent_started"})
            };
            writeln!(stream, "{}", json!({"id":wire["id"],"result":result})).unwrap();
        })
    }
    fn assert_no_partial_bytes(frames: &[Value], received: usize) {
        assert_eq!(
            received,
            frames
                .iter()
                .map(|frame| frame.to_string().len() + 1)
                .sum::<usize>(),
            "uncaptured operation bytes were written"
        );
    }

    #[test]
    fn process_hint_prewrite_fences_are_not_submitted() {
        for case in [
            "start-time",
            "peer",
            "socket",
            "epoch",
            "incarnation",
            "expired",
            "cancelled",
            "expired-before",
            "cancelled-before",
            "connect",
            "encoding",
        ] {
            let fixture = fixture(pong());
            let clock = HintClock(Arc::new(AtomicU64::new(0)));
            let budget = CallBudget {
                deadline: MonoInstant(2000),
                cancellation: Cancellation::default(),
            };
            let provider = ChangingProvider {
                calls: AtomicUsize::new(0),
                change_at: if matches!(case, "start-time" | "peer") {
                    2
                } else {
                    usize::MAX
                },
                peer: case == "peer",
            };
            let validations = AtomicUsize::new(0);
            let validate = |_: &Value, _: &LocalEndpointWitness| {
                let call = validations.fetch_add(1, Ordering::AcqRel);
                match (case, call) {
                    ("socket", 0) => fixture.rebind(),
                    ("connect", 0) => fixture.stop_listening(),
                    ("incarnation", 0) | ("epoch", 1) => {
                        return Err(error(
                            ErrorCode::StaleHostObservation,
                            "injected admitted fence changed",
                        ));
                    }
                    ("expired", 1) => clock.0.store(2000, Ordering::Release),
                    ("cancelled", 1) => budget.cancellation.cancel(),
                    _ => {}
                }
                Ok(())
            };
            if case == "expired-before" {
                clock.0.store(2000, Ordering::Release);
            }
            if case == "cancelled-before" {
                budget.cancellation.cancel();
            }
            let mut request = params();
            if case == "encoding" {
                request["args"] = json!(["x".repeat(MAX_REQUEST)]);
            }
            let result = guarded_start_inner(
                fixture.endpoint(),
                "guarded",
                request,
                &clock,
                &budget,
                Duration::from_secs(30),
                &provider,
                &validate,
            );
            let (frames, received) = fixture.finish_with_bytes();
            assert_eq!(
                frames
                    .iter()
                    .filter(|frame| frame["method"] == "agent.start")
                    .count(),
                0,
                "{case} wrote an operation"
            );
            assert_no_partial_bytes(&frames, received);
            let failure = result.expect_err(case);
            assert_eq!(failure.submission, NativeSubmission::NotSubmitted, "{case}");
            let code = match case {
                "expired" | "expired-before" => ErrorCode::DeadlineExceeded,
                "cancelled" | "cancelled-before" => ErrorCode::Cancelled,
                "connect" => ErrorCode::HostUnavailable,
                "encoding" => ErrorCode::InvalidRequest,
                _ => ErrorCode::StaleHostObservation,
            };
            assert_eq!(failure.error.code, code, "{case}");
        }
    }

    #[test]
    fn process_hint_shared_exact_version_protocol_gate_remains_separate() {
        for (version, protocol, supported) in [
            ("0.9.1", 22, true),
            ("0.9.3", 22, true),
            ("0.9.2", 22, false),
            ("0.9.10", 22, false),
            ("0.9.1-preview", 22, false),
            ("0.9.3-preview", 22, false),
            ("0.9.1", 21, false),
            ("0.9.3", 23, false),
        ] {
            let mut advert = pong();
            advert["version"] = json!(version);
            advert["protocol"] = json!(protocol);
            let fixture = fixture(advert);
            let clock = HintClock(Arc::new(AtomicU64::new(0)));
            let budget = CallBudget {
                deadline: MonoInstant(2000),
                cancellation: Cancellation::default(),
            };
            let result = request_guarded_start(
                fixture.endpoint(),
                "guarded",
                params(),
                &clock,
                &budget,
                Duration::from_secs(30),
                &|_, _| Ok(()),
            );
            let (frames, received) = fixture.finish_with_bytes();
            assert_no_partial_bytes(&frames, received);
            assert_eq!(
                frames
                    .iter()
                    .filter(|frame| frame["method"] == "agent.start")
                    .count(),
                usize::from(supported),
                "{version}/{protocol}"
            );
            if supported {
                result.unwrap();
            } else {
                let failure = result.unwrap_err();
                assert_eq!(failure.error.code, ErrorCode::Unsupported);
                assert_eq!(failure.submission, NativeSubmission::NotSubmitted);
            }
        }
    }

    #[test]
    fn process_hint_transport_serialization_preserves_empty_token_only_at_private_boundary() {
        // Serialization evidence only: NativeLaunchRequest still rejects empty elements.
        let fixture = fixture(pong());
        let clock = HintClock(Arc::new(AtomicU64::new(0)));
        let budget = CallBudget {
            deadline: MonoInstant(2000),
            cancellation: Cancellation::default(),
        };
        let mut request = params();
        request["args"] = json!(["", "space arg", "apostrophe's arg"]);
        let result = request_guarded_start(
            fixture.endpoint(),
            "guarded",
            request.clone(),
            &clock,
            &budget,
            Duration::from_secs(30),
            &|_, _| Ok(()),
        );
        let (frames, received) = fixture.finish_with_bytes();
        assert_no_partial_bytes(&frames, received);
        result.unwrap();
        let starts: Vec<_> = frames
            .iter()
            .filter(|frame| frame["method"] == "agent.start")
            .collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0]["params"], request);
    }

    #[test]
    fn process_hint_postwrite_uncertainty_is_possible_and_exact_refusal_is_not_submitted() {
        for case in [
            "malformed",
            "eof",
            "timeout",
            "cancelled",
            "witness",
            "unsupported-exact",
            "unsupported-lookalike",
            "other-unsupported",
        ] {
            let clock = HintClock(Arc::new(AtomicU64::new(0)));
            let budget = CallBudget {
                deadline: MonoInstant(2000),
                cancellation: Cancellation::default(),
            };
            let reply_clock = clock.0.clone();
            let cancellation = budget.cancellation.clone();
            let fixture = HintSocketFixture::new(move |stream, wire| {
                if wire["method"] == "ping" {
                    writeln!(stream, "{}", json!({"id":wire["id"],"result":pong()})).unwrap();
                    return;
                }
                assert_eq!(wire["method"], "agent.start");
                match case {
                    "malformed" => {
                        writeln!(stream, "{{invalid").unwrap();
                        return;
                    }
                    "eof" => return,
                    "timeout" => reply_clock.store(2000, Ordering::Release),
                    "cancelled" => cancellation.cancel(),
                    _ => {}
                }
                let response = match case {
                    "unsupported-exact" => {
                        json!({"error":{"code":"agent_process_hint_unsupported","message":"unsupported shell"}})
                    }
                    "unsupported-lookalike" => {
                        json!({"error":{"code":"agent_process_hint_unsupported_other","message":"agent_process_hint_unsupported"}})
                    }
                    "other-unsupported" => {
                        json!({"error":{"code":"unsupported_version","message":"unsupported"}})
                    }
                    _ => json!({"result":{"type":"agent_started"}}),
                };
                let mut response = response;
                response["id"] = wire["id"].clone();
                let _ = writeln!(stream, "{response}");
            });
            let provider = ChangingProvider {
                calls: AtomicUsize::new(0),
                change_at: if case == "witness" { 3 } else { usize::MAX },
                peer: false,
            };
            let result = guarded_start_inner(
                fixture.endpoint(),
                "guarded",
                params(),
                &clock,
                &budget,
                Duration::from_secs(30),
                &provider,
                &|_, _| Ok(()),
            );
            let (frames, received) = fixture.finish_with_bytes();
            assert_eq!(
                frames
                    .iter()
                    .filter(|frame| frame["method"] == "agent.start")
                    .count(),
                1,
                "{case}"
            );
            assert_no_partial_bytes(&frames, received);
            let failure = result.expect_err(case);
            assert_eq!(
                failure.submission,
                if case == "unsupported-exact" {
                    NativeSubmission::NotSubmitted
                } else {
                    NativeSubmission::Possible
                },
                "{case}"
            );
            if case == "unsupported-exact" {
                assert_eq!(failure.error.code, ErrorCode::Unsupported);
            }
        }
    }
}
/// Existing callers retain their original transport and capability behavior.
pub(crate) fn request(
    socket: &Path,
    id: &str,
    method: &str,
    params: Value,
    clock: &dyn Clock,
    budget: &CallBudget,
    limit: Duration,
) -> Result<String, ApiError> {
    request_inner(socket, id, method, params, clock, budget, limit, None).map(|(body, _)| body)
}

/// Each actual stream is checked before sending and after correlated response
/// EOF; ping and operation must have equal witnesses within one call budget.
pub fn request_witnessed(
    socket: &Path,
    id: &str,
    method: &str,
    params: Value,
    clock: &dyn Clock,
    budget: &CallBudget,
    limit: Duration,
) -> Result<WitnessedResponse, ApiError> {
    let (body, witness) = request_inner(
        socket,
        id,
        method,
        params,
        clock,
        budget,
        limit,
        Some(&KernelProcessInfo),
    )?;
    let witness = witness.ok_or_else(|| {
        error(
            ErrorCode::StaleHostObservation,
            "endpoint witness unavailable",
        )
    })?;
    Ok(WitnessedResponse { body, witness })
}
fn witness_error(error_value: super::continuity::CaptureError) -> ApiError {
    let code = if error_value == super::continuity::CaptureError::Unsupported {
        ErrorCode::Unsupported
    } else {
        ErrorCode::StaleHostObservation
    };
    // A missing or non-socket endpoint is Herdr not running (never started,
    // or its socket removed): say so in the same words as a refused connect.
    let prefix = if error_value == super::continuity::CaptureError::EndpointUnavailable {
        "server not running: "
    } else {
        ""
    };
    error(
        code,
        format!("{prefix}host endpoint witness unavailable: {error_value:?}"),
    )
}

#[cfg(all(test, target_os = "macos"))]
#[path = "../../tests/host/witness_transport.rs"]
mod witness_tests;
