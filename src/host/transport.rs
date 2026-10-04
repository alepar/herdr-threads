//! One bounded Herdr 0.9.1 API exchange. Dropping this future closes its socket.

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
    let mut position = 0;
    while position < request.len() {
        let count = bounded(
            stream.write(&request[position..]),
            clock,
            budget,
            started,
            limit,
        )
        .await?;
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
                    )
                    .await?;
                    if let Some(error) = super::observation::structured_host_error(&ping) {
                        return Err(error);
                    }
                    let pong = ping
                        .get("result")
                        .ok_or_else(|| error(ErrorCode::Unsupported, "host API ping failed"))?;
                    if pong.get("type").and_then(Value::as_str) != Some("pong")
                        || pong.get("protocol").and_then(Value::as_u64) != Some(22)
                        || pong.get("version").and_then(Value::as_str) != Some("0.9.1")
                    {
                        return Err(error(
                            ErrorCode::Unsupported,
                            "host API version or protocol mismatch",
                        ));
                    }
                    let (result, witness) = exchange(
                        socket, id, method, params, clock, budget, started, limit, provider,
                    )
                    .await?;
                    if ping_witness != witness {
                        return Err(error(
                            ErrorCode::StaleHostObservation,
                            "host endpoint changed between ping and operation",
                        ));
                    }
                    if let Some(error) = super::observation::structured_host_error(&result) {
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
                    if expected.is_some_and(|expected| {
                        result.pointer("/result/type").and_then(Value::as_str) != Some(expected)
                    }) {
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
