//! Bounded local IPC: ordinary requests are one-shot; registered service sessions persist.

mod service_connection;
pub(crate) use service_connection::LiveServiceGate;

use crate::protocol::{
    authority::PeerIdentity,
    commands::Command,
    results::{ApiError, ErrorCode},
    service::ServiceWireRequest,
    time::{CallBudget, Cancellation, Clock, MonoInstant},
    wire::{
        MAX_WIRE_FRAME_BYTES, PROTOCOL_VERSION, WireRequest, WireResponse, encode_wire_response,
    },
};
use crate::{daemon::ownership::OwnedAsyncListener, ports::LocalService};
use serde::Serialize;
use std::{io, io::Write, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::UnixStream,
    sync::Semaphore,
    task::JoinHandle,
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
    tasks: Vec<JoinHandle<io::Result<()>>>,
    next: usize,
    first_error: Option<io::Error>,
}
impl PendingDrain {
    pub fn remaining(&self) -> usize {
        self.tasks.iter().filter(|task| !task.is_finished()).count()
    }

    /// Safe to cancel and call again: the handle and owner lease stay in `self`.
    pub async fn wait(&mut self) -> io::Result<()> {
        while self.next < self.tasks.len() {
            let result = (&mut self.tasks[self.next])
                .await
                .map_err(io::Error::other)
                .and_then(|result| result);
            if self.first_error.is_none() {
                self.first_error = result.err();
            }
            self.next += 1;
        }
        self.first_error.take().map_or(Ok(()), Err)
    }
}

async fn cancelled(shutdown: &Cancellation) {
    while !shutdown.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
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
    ApiError {
        code,
        detail: detail.to_owned(),
        restart_argv: None,
        required_minimum_bytes: None,
    }
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
    let search_slots = Arc::new(Semaphore::new(5));
    let active_search = Arc::new(Semaphore::new(1));
    let mut tasks = Vec::new();
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
            _ = cancelled(&shutdown) => break,
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
        let active_search = active_search.clone();
        let instance = instance.clone();
        let daemon_boot = daemon_boot.clone();
        let service_gate = service_gate.clone();
        tasks.retain(|task: &JoinHandle<io::Result<()>>| !task.is_finished());
        tasks.push(tokio::spawn(async move {
            let _permit = permit;
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
            )
            .await
        }));
    }
    let drain_deadline = tokio::time::Instant::now() + SHUTDOWN_DRAIN;
    loop {
        tasks.retain(|task| !task.is_finished());
        if tasks.is_empty() {
            return match accept_error {
                Some(error) => Err(error),
                None => Ok(ServeOutcome::Drained),
            };
        }
        if tokio::time::Instant::now() >= drain_deadline {
            return Ok(ServeOutcome::Incomplete(PendingDrain {
                _listener: listener,
                tasks,
                next: 0,
                first_error: accept_error,
            }));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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
    service_gate: Arc<service_connection::LiveServiceGate>,
) -> io::Result<()> {
    let uid = stream.peer_cred()?.uid();
    if uid != owner_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "peer UID differs from owner",
        ));
    }
    let peer = PeerIdentity::from_kernel(uid);
    let expires = tokio::time::Instant::now() + ORDINARY_TIMEOUT;
    let frame = tokio::select! {
        result = tokio::time::timeout_at(expires, read_frame(&mut stream)) =>
            result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "request expired"))??,
        _ = cancelled(&shutdown) => return Ok(()),
    };
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
    let request =
        WireRequest::decode(&frame).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
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
                _ = cancelled(&shutdown) => None,
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
            _ = cancelled(&shutdown), if !is_stop => {
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
