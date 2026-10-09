//! Bounded local IPC: ordinary requests are one-shot; registered service sessions persist.

mod service_connection;
mod watch_connection;

use crate::protocol::{
    authority::PeerIdentity,
    commands::Command,
    results::{ApiError, ErrorCode},
    service::ServiceWireRequest,
    time::{CallBudget, Cancellation, Clock, MonoInstant},
    watch::{MAX_WATCH_CONNECTIONS, WatchRefusalReason, WatchWireRequest},
    wire::{
        MAX_WIRE_FRAME_BYTES, PROTOCOL_VERSION, WireRequest, WireResponse, encode_wire_response,
    },
};
use crate::{
    daemon::ownership::OwnedAsyncListener, ports::LocalService, service::live_gate::LiveServiceGate,
};
use serde::Serialize;
use std::{io, io::Write, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::UnixStream,
    sync::Semaphore,
    task::JoinSet,
};
use uuid::Uuid;

pub const MAX_FRAME_BYTES: usize = MAX_WIRE_FRAME_BYTES - 4;
pub const MAX_CONNECTIONS: usize = 32;
const ORDINARY_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_DRAIN: Duration = Duration::from_millis(250);

/// Incomplete shutdown retains the listener and every accepted task. The daemon
/// owner must retain this value (and its ownership lease) until `wait` completes.
#[must_use]
pub enum ServeOutcome {
    Drained,
    Incomplete(PendingDrain),
}

#[must_use]
pub struct PendingDrain {
    _listener: OwnedAsyncListener,
    tasks: JoinSet<io::Result<()>>,
    first_error: Option<io::Error>,
}
impl PendingDrain {
    pub fn remaining(&self) -> usize {
        self.tasks.len()
    }

    /// Safe to cancel and call again: the task set and owner lease stay in `self`.
    pub async fn wait(&mut self) -> io::Result<()> {
        while let Some(joined) = self.tasks.join_next().await {
            let result = joined.map_err(io::Error::other).and_then(|result| result);
            if self.first_error.is_none() {
                self.first_error = result.err();
            }
        }
        self.first_error.take().map_or(Ok(()), Err)
    }
}
impl Drop for PendingDrain {
    /// Dropping a `JoinSet` aborts its tasks; a dropped drain detaches them,
    /// as the handle vector it replaced did.
    fn drop(&mut self) {
        self.tasks.detach_all();
    }
}

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Vec<u8>> {
    let length = reader.read_u32().await? as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid frame length",
        ));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    Ok(body)
}

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, body: &[u8]) -> io::Result<()> {
    if body.is_empty() || body.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid frame length",
        ));
    }
    writer.write_all(&(body.len() as u32).to_be_bytes()).await?;
    writer.write_all(body).await
}

fn api_error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError::new(code, detail.to_owned())
}

struct BoundedJson(Vec<u8>);
impl Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_FRAME_BYTES - self.0.len() {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "response frame full",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn encode_json<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    let mut writer = BoundedJson(Vec::with_capacity(MAX_FRAME_BYTES));
    serde_json::to_writer(&mut writer, value).map_err(|error| {
        if error.io_error_kind() == Some(io::ErrorKind::OutOfMemory) {
            io::Error::new(io::ErrorKind::OutOfMemory, "frame exceeds limit")
        } else {
            io::Error::other(error)
        }
    })?;
    Ok(writer.0)
}

fn encode_response(response: &WireResponse) -> io::Result<Vec<u8>> {
    match encode_wire_response(response) {
        Ok(frame) => Ok(frame),
        Err(error) if error.code == ErrorCode::InvalidBudget => {
            let fallback = WireResponse {
                version: PROTOCOL_VERSION,
                request_id: response.request_id.clone(),
                instance: response.instance.clone(),
                daemon_boot: response.daemon_boot.clone(),
                result: Err(error),
            };
            encode_wire_response(&fallback).map_err(|error| io::Error::other(error.detail))
        }
        Err(error) => Err(io::Error::other(error.detail)),
    }
}

/// The listener and owner UID come from the ownership layer. This function never
/// binds or removes a socket path and cannot elect a daemon owner.
pub async fn serve(
    listener: OwnedAsyncListener,
    instance_uuid: Uuid,
    handler: Arc<dyn LocalService>,
    clock: Arc<dyn Clock>,
    owner_uid: u32,
    shutdown: Cancellation,
) -> io::Result<ServeOutcome> {
    serve_with_gate(
        listener,
        instance_uuid,
        handler,
        clock,
        owner_uid,
        shutdown,
        Arc::new(LiveServiceGate::new()),
    )
    .await
}

/// B2 can share this gate with generation-targeted recovery and diagnostics.
/// The default `serve` path owns a fresh gate per daemon boot.
pub(crate) async fn serve_with_gate(
    listener: OwnedAsyncListener,
    instance_uuid: Uuid,
    handler: Arc<dyn LocalService>,
    clock: Arc<dyn Clock>,
    owner_uid: u32,
    shutdown: Cancellation,
    service_gate: Arc<LiveServiceGate>,
) -> io::Result<ServeOutcome> {
    let instance = instance_uuid.to_string();
    let daemon_boot = listener.boot_id().to_string();
    let connections = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    // Watch connections have their own admission, outside MAX_CONNECTIONS.
    let watch_slots = Arc::new(Semaphore::new(MAX_WATCH_CONNECTIONS));
    let search_slots = Arc::new(Semaphore::new(5));
    let active_search = Arc::new(Semaphore::new(1));
    let mut tasks: JoinSet<io::Result<()>> = JoinSet::new();
    let mut accept_error = None;
    while !shutdown.is_cancelled() {
        let accepted = tokio::select! {
            result = listener.accept() => match result {
                Ok(accepted) => accepted,
                Err(error) => {
                    accept_error = Some(error);
                    shutdown.cancel();
                    break;
                }
            },
            _ = shutdown.cancelled() => break,
        };
        let Ok(permit) = connections.clone().try_acquire_owned() else {
            drop(accepted);
            continue;
        };
        let (stream, _) = accepted;
        let handler = handler.clone();
        let clock = clock.clone();
        let shutdown = shutdown.clone();
        let search_slots = search_slots.clone();
        let watch_slots = watch_slots.clone();
        let active_search = active_search.clone();
        let instance = instance.clone();
        let daemon_boot = daemon_boot.clone();
        let service_gate = service_gate.clone();
        while tasks.try_join_next().is_some() {}
        tasks.spawn(async move {
            serve_connection(
                stream,
                instance,
                daemon_boot,
                handler,
                clock,
                owner_uid,
                shutdown,
                search_slots,
                active_search,
                service_gate,
                permit,
                watch_slots,
            )
            .await
        });
    }
    let drain_deadline = tokio::time::Instant::now() + SHUTDOWN_DRAIN;
    // Wake on each finishing task; the deadline bounds the whole drain.
    while !tasks.is_empty() {
        tokio::select! {
            _ = tasks.join_next() => {}
            _ = tokio::time::sleep_until(drain_deadline) => {
                return Ok(ServeOutcome::Incomplete(PendingDrain {
                    _listener: listener,
                    tasks,
                    first_error: accept_error,
                }));
            }
        }
    }
    match accept_error {
        Some(error) => Err(error),
        None => Ok(ServeOutcome::Drained),
    }
}

/// The minimal skew answer: when `frame` is a JSON object whose `version` is
/// a wire version this daemon does not speak and whose `request_id` is a
/// valid id, a `WireResponse` echoing both (so the sender's own correlation
/// accepts it) carrying `UnknownWireVersion` with the skew remedy. Reads two
/// fields only; nothing else of the foreign request is decoded.
fn skew_reply(frame: &[u8], instance: &str, daemon_boot: &str) -> Option<WireResponse> {
    let value: serde_json::Value = serde_json::from_slice(frame).ok()?;
    let object = value.as_object()?;
    let version = u16::try_from(object.get("version")?.as_u64()?).ok()?;
    if version == PROTOCOL_VERSION {
        return None;
    }
    let request_id = object.get("request_id")?.as_str()?;
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id.bytes().all(|b| b.is_ascii_graphic())
    {
        return None;
    }
    let detail = crate::daemon::remedy::remedy(
        Some(crate::protocol::results::ErrorClass::VersionSkew),
        &crate::daemon::remedy::RemedyContext::VersionSkew {
            daemon: format!(
                "{} (protocol {PROTOCOL_VERSION})",
                env!("CARGO_PKG_VERSION")
            ),
            cli: format!("protocol {version}"),
        },
    );
    Some(WireResponse {
        version,
        request_id: request_id.to_owned(),
        instance: instance.to_owned(),
        daemon_boot: daemon_boot.to_owned(),
        result: Err(api_error(ErrorCode::UnknownWireVersion, &detail)),
    })
}

// Allowed: per-connection state handed over from the accept loop.
#[allow(clippy::too_many_arguments)]
async fn serve_connection(
    mut stream: UnixStream,
    instance: String,
    daemon_boot: String,
    handler: Arc<dyn LocalService>,
    clock: Arc<dyn Clock>,
    owner_uid: u32,
    shutdown: Cancellation,
    search_slots: Arc<Semaphore>,
    active_search: Arc<Semaphore>,
    service_gate: Arc<LiveServiceGate>,
    permit: tokio::sync::OwnedSemaphorePermit,
    watch_slots: Arc<Semaphore>,
) -> io::Result<()> {
    // The ordinary admission permit lives until this function returns, except
    // on a watch connection, which hands it back once sniffed.
    let mut ordinary_permit = Some(permit);
    let uid = stream.peer_cred()?.uid();
    if uid != owner_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "peer UID differs from owner",
        ));
    }
    let peer = PeerIdentity::from_kernel(uid);
    let expires =
        tokio::time::Instant::now() + crate::protocol::time::external_bound(ORDINARY_TIMEOUT);
    let frame = tokio::select! {
        result = tokio::time::timeout_at(expires, read_frame(&mut stream)) =>
            result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "request expired"))??,
        _ = shutdown.cancelled() => return Ok(()),
    };
    if let Ok(request) = WatchWireRequest::decode(&frame) {
        drop(ordinary_permit.take());
        let Ok(watch_permit) = watch_slots.try_acquire_owned() else {
            return watch_connection::refuse(
                stream,
                &request,
                &instance,
                &daemon_boot,
                WatchRefusalReason::Busy,
            )
            .await;
        };
        return watch_connection::serve(
            stream,
            request,
            watch_permit,
            instance,
            daemon_boot,
            handler,
            clock,
            shutdown,
        )
        .await;
    }
    if let Ok(request) = serde_json::from_slice::<ServiceWireRequest>(&frame) {
        return service_connection::serve_registered(
            stream,
            request,
            expires,
            instance,
            daemon_boot,
            handler,
            clock,
            service_gate,
            shutdown,
        )
        .await;
    }
    let request = match WireRequest::decode(&frame) {
        Ok(request) => request,
        Err(error) => {
            // A request of another wire version gets a reply its sender's
            // decoder understands instead of a closed socket (root §B3 D3).
            if let Some(response) = skew_reply(&frame, &instance, &daemon_boot) {
                let frame = encode_response(&response)?;
                return tokio::time::timeout_at(expires, stream.write_all(&frame))
                    .await
                    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "response expired"))?;
            }
            return Err(io::Error::new(io::ErrorKind::InvalidData, error));
        }
    };
    if request.expected_instance != instance {
        let response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id,
            instance,
            daemon_boot,
            result: Err(api_error(
                ErrorCode::InstanceMismatch,
                "request instance differs from owner",
            )),
        };
        let frame = encode_response(&response)?;
        return tokio::time::timeout_at(expires, stream.write_all(&frame))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "response expired"))?;
    }
    if let Some(expected) = request
        .expected_boot
        .as_deref()
        .filter(|expected| *expected != daemon_boot)
    {
        let detail = format!(
            "request expected daemon boot {expected}; this daemon is boot {daemon_boot}; nothing was applied"
        );
        let response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id,
            instance,
            daemon_boot,
            result: Err(api_error(ErrorCode::DaemonBootChanged, &detail)),
        };
        let frame = encode_response(&response)?;
        return tokio::time::timeout_at(expires, stream.write_all(&frame))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "response expired"))?;
    }
    if request.request_id.len() > 128 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request id too long",
        ));
    }
    let is_search = matches!(request.command, Command::Search(_));
    let is_stop = matches!(request.command, Command::Stop(_));
    let is_service_control = matches!(
        request.command,
        Command::ServiceInspect | Command::ServiceDisconnect(_)
    );
    #[cfg(feature = "test-support")]
    let mut _permit_exit_watch =
        crate::test_support::server_completion::SearchPermitsReturned::new(&request.command);
    let slot = if is_search {
        search_slots.clone().try_acquire_owned().ok()
    } else {
        None
    };
    #[cfg(feature = "test-support")]
    _permit_exit_watch.arm(slot.is_some());
    #[cfg(feature = "test-support")]
    if slot.is_some()
        && let Command::Search(query) = &request.command
    {
        crate::test_support::search_barrier::note_admitted(&query.literal);
    }
    let result = if is_search && slot.is_none() {
        Err(api_error(ErrorCode::StoreBusy, "search queue full"))
    } else {
        let cancellation = Cancellation::default();
        let budget = CallBudget {
            deadline: MonoInstant(
                clock.monotonic_now().0.saturating_add(
                    expires
                        .saturating_duration_since(tokio::time::Instant::now())
                        .as_millis() as u64,
                ),
            ),
            cancellation: cancellation.clone(),
        };
        let (mut read_half, mut write_half) = stream.into_split();
        let _active = if is_search {
            let acquire = active_search.clone().acquire_owned();
            tokio::select! {
                acquired = acquire => acquired.ok(),
                _ = tokio::time::sleep_until(expires) => None,
                _ = read_half.read_u8() => None,
                _ = shutdown.cancelled() => None,
            }
        } else {
            None
        };
        if is_search && _active.is_none() {
            cancellation.cancel();
            return Ok(());
        }
        if shutdown.is_cancelled() || tokio::time::Instant::now() >= expires {
            cancellation.cancel();
            return Ok(());
        }
        let command = request.command;
        let output = request.output.unwrap_or_default();
        let control_instance = instance.clone();
        let control_boot = daemon_boot.clone();
        let mut work = tokio::task::spawn_blocking(move || {
            #[cfg(feature = "test-support")]
            let _worker_exit_watch =
                crate::test_support::server_completion::WorkerExit::enter(&command);
            if is_service_control {
                if output != crate::protocol::output::OutputSpec::default() {
                    return Err(api_error(
                        ErrorCode::Unsupported,
                        "selected output is unavailable for service recovery",
                    ));
                }
                handler.service_control(
                    command,
                    peer,
                    &control_instance,
                    &control_boot,
                    service_gate.as_ref(),
                    &budget,
                )
            } else {
                handler.handle_with_output(command, peer, &budget, &output)
            }
        });
        let result = tokio::select! {
            joined = &mut work => Some(joined.map_err(io::Error::other)?),
            _ = tokio::time::sleep_until(expires) => {
                cancellation.cancel();
                None
            }
            changed = read_half.read_u8() => {
                let _ = changed;
                cancellation.cancel();
                None
            }
            _ = shutdown.cancelled(), if !is_stop => {
                cancellation.cancel();
                None
            }
        };
        let Some(result) = result else {
            // A synchronous handler can still be in its decision path. Keep all
            // admission permits until its thread has actually exited.
            let _ = work.await;
            return Ok(());
        };
        let response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id,
            instance,
            daemon_boot,
            result,
        };
        let frame = encode_response(&response)?;
        tokio::time::timeout_at(expires, write_half.write_all(&frame))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "response expired"))??;
        return Ok(());
    };
    let response = WireResponse {
        version: PROTOCOL_VERSION,
        request_id: request.request_id,
        instance,
        daemon_boot,
        result,
    };
    let frame = encode_response(&response)?;
    tokio::time::timeout_at(expires, stream.write_all(&frame))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "response expired"))?
}

#[cfg(test)]
#[path = "../../tests/daemon/transport.rs"]
mod tests;
