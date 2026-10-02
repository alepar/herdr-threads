//! Owner-scoped child diagnostics lifecycle.

use crate::client::local::LocalSocketClient;
use crate::daemon::diagnostics::{
    BufferLimits, DiagnosticBuffer, DiagnosticSink, DiagnosticSource,
};
use crate::daemon::ownership::{
    EndpointDescriptor, OwnerLock, owner_lock_identity, previous_owner_released, read_descriptor,
    read_existing_namespace,
};
use crate::daemon::paths::{InstancePaths, RuntimeContext, effective_uid};
use crate::daemon::transport::{self, ServeOutcome};
use crate::ports::LocalService;
use crate::protocol::{
    commands::Command,
    results::{ApiError, CommandResult, ErrorCode, HealthState},
    time::{CallBudget, Cancellation, Clock, MonoInstant},
    wire::PROTOCOL_VERSION,
};
use std::{
    future::Future,
    io,
    os::unix::process::CommandExt,
    path::Path,
    process::{Command as ProcessCommand, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

/// The ownership module supplies the real lock guard; holding it keeps the sink owner elected.
pub struct OwnerSession<S: DiagnosticSink> {
    diagnostics: DiagnosticBuffer<S>,
    lease: OwnerLock,
    admitting: bool,
}

/// A losing starter passes `None`; its sink is never installed.
pub fn start_after_election<S: DiagnosticSink>(
    lease: Option<OwnerLock>,
    sink: S,
    limits: BufferLimits,
) -> io::Result<Option<OwnerSession<S>>> {
    let Some(lease) = lease else {
        return Ok(None);
    };
    let diagnostics = DiagnosticBuffer::install(sink, limits)?;
    Ok(Some(OwnerSession {
        diagnostics,
        lease,
        admitting: true,
    }))
}

impl<S: DiagnosticSink> OwnerSession<S> {
    pub fn emit(&mut self, source: DiagnosticSource, fragment: &[u8]) -> io::Result<()> {
        if !self.admitting {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "daemon stopping"));
        }
        self.diagnostics.emit(source, fragment)
    }
    pub fn stop_admission(&mut self) {
        self.admitting = false;
    }
    pub(crate) fn drain_and_close(&mut self) -> io::Result<()> {
        self.stop_admission();
        self.diagnostics.drain_and_close()
    }
    pub fn owner_lease(&self) -> &OwnerLock {
        &self.lease
    }
    pub fn flush(&mut self) -> io::Result<()> {
        self.diagnostics.flush()
    }
    pub(crate) fn max_fragment_bytes(&self) -> usize {
        self.diagnostics.max_fragment_bytes()
    }
    pub(crate) fn discard_bytes(&mut self, bytes: usize) {
        self.diagnostics.discard(bytes);
    }
    pub fn buffered_bytes(&self) -> usize {
        self.diagnostics.buffered_bytes()
    }
    pub fn dropped_bytes(&self) -> usize {
        self.diagnostics.dropped_bytes()
    }
}

impl<S: DiagnosticSink> Drop for OwnerSession<S> {
    fn drop(&mut self) {
        let _ = self.drain_and_close();
    }
}

const ENSURE_WAIT: Duration = Duration::from_secs(5);

/// Names the pid of the test process that owns any daemon started under it.
/// Inherited through `daemon ensure`'s detached spawn; honored only by
/// `test-support` builds (see `test_support::owner_watch`), ignored otherwise.
pub const TEST_OWNER_PID_ENV: &str = "HERDR_THREADS_TEST_OWNER_PID";

/// `(TEST_OWNER_PID_ENV, <this process id>)`, for tests to pass with
/// `Command::envs([test_owner_env()])` on anything that may start a daemon.
pub fn test_owner_env() -> (&'static str, String) {
    (TEST_OWNER_PID_ENV, std::process::id().to_string())
}

fn record_owner_error<S: DiagnosticSink>(owner: &mut OwnerSession<S>, error: &io::Error) {
    let detail = format!("daemon owner error: {error}\n");
    let _ = owner.emit(DiagnosticSource::Daemon, detail.as_bytes());
    let _ = owner.flush();
}

fn api_error(code: ErrorCode, detail: impl Into<String>) -> ApiError {
    ApiError {
        code,
        detail: detail.into(),
        restart_argv: None,
        required_minimum_bytes: None,
    }
}

/// The definite refusal for a daemon whose published protocol differs from
/// this executable's. Shared by `ensure` and every CLI client so none of them
/// sends a request an older daemon would drop at decode.
pub(crate) fn protocol_mismatch_error(daemon_protocol: u16) -> ApiError {
    api_error(
        ErrorCode::UnknownWireVersion,
        format!(
            "daemon protocol {daemon_protocol} differs from executable protocol {PROTOCOL_VERSION}; run `daemon stop` with the matching older executable/protocol, then `daemon ensure` with the new executable and the same state/host context"
        ),
    )
}

fn io_error(error: io::Error) -> ApiError {
    let code = if crate::daemon::paths::is_unsafe_local_state(&error) {
        ErrorCode::InvalidRequest
    } else {
        ErrorCode::HostUnavailable
    };
    api_error(code, format!("daemon startup: {error}"))
}

fn instance_uuid(paths: &InstancePaths) -> io::Result<Option<Uuid>> {
    read_existing_namespace(paths)
}

async fn handshake(
    paths: &InstancePaths,
    clock: Arc<dyn Clock>,
    remaining: Duration,
) -> Result<Option<EndpointDescriptor>, ApiError> {
    let Some(instance) = instance_uuid(paths).map_err(io_error)? else {
        return Ok(None);
    };
    let descriptor = match read_descriptor(paths, instance) {
        Ok(descriptor) => descriptor,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    if descriptor.protocol_version != PROTOCOL_VERSION {
        let lock_identity = owner_lock_identity(paths).map_err(io_error)?;
        if previous_owner_released(paths, lock_identity).map_err(io_error)? {
            return Ok(None);
        }
        let current = match read_descriptor(paths, instance) {
            Ok(current) => current,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        if current != descriptor
            || previous_owner_released(paths, lock_identity).map_err(io_error)?
        {
            return Ok(None);
        }
        return Err(protocol_mismatch_error(descriptor.protocol_version));
    }
    let client = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        clock.clone(),
        instance,
        Some(descriptor.boot_id),
    );
    let budget = CallBudget {
        deadline: MonoInstant(
            clock
                .monotonic_now()
                .0
                .saturating_add(remaining.as_millis() as u64),
        ),
        cancellation: Cancellation::default(),
    };
    let result = match client.call_async(Command::Health, &budget).await {
        Ok(result) => result,
        // A sandbox refusing the socket is not a missing daemon: starting
        // another owner cannot help, so surface it instead of waiting.
        Err(error) if error.code == ErrorCode::TransportDenied => return Err(error),
        Err(_) => return Ok(None),
    };
    let CommandResult::Health(health) = result else {
        return Err(api_error(
            ErrorCode::HostUnavailable,
            "daemon health handshake returned another result",
        ));
    };
    if health.protocol_version != PROTOCOL_VERSION {
        return Err(api_error(
            ErrorCode::UnknownWireVersion,
            "daemon health protocol differs from endpoint; run `daemon stop` then `daemon ensure`",
        ));
    }
    if health.instance_id != instance.to_string()
        || health.boot_id != descriptor.boot_id.to_string()
        || health.validate().is_err()
    {
        return Err(api_error(
            ErrorCode::HostUnavailable,
            "daemon health identity differs from endpoint",
        ));
    }
    if health.software_version != env!("CARGO_PKG_VERSION")
        || descriptor.software_version != env!("CARGO_PKG_VERSION")
    {
        return Err(api_error(
            ErrorCode::DaemonVersionMismatch,
            format!(
                "daemon version {} differs from executable {}; run `daemon stop` then `daemon ensure`",
                health.software_version,
                env!("CARGO_PKG_VERSION")
            ),
        ));
    }
    // A reachable, identity-matching daemon that reports Degraded (for
    // example unsupported native capabilities) is running and serving; the
    // caller prints its health. Only Unavailable is a startup failure.
    if health.state == HealthState::Unavailable {
        return Err(api_error(
            ErrorCode::HostUnavailable,
            "daemon is reachable but unavailable; inspect `daemon health` and logs",
        ));
    }
    Ok(Some(descriptor))
}

fn spawn_detached(executable: &Path, context: &RuntimeContext) -> io::Result<()> {
    let mut command = ProcessCommand::new(executable);
    command
        .arg("daemon")
        .arg("run")
        .arg("--state-dir")
        .arg(&context.state_dir)
        .arg("--host-endpoint")
        .arg(&context.host_endpoint)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(path) = &context.herdr_bin {
        command.env("HERDR_BIN_PATH", path);
    } else {
        command.env_remove("HERDR_BIN_PATH");
    }
    command.process_group(0);
    command.spawn().map(|_| ())
}

/// Probe first; a live lock holder is never displaced even when its socket is silent.
pub async fn ensure_running(
    context: &RuntimeContext,
    executable: &Path,
    clock: Arc<dyn Clock>,
) -> Result<EndpointDescriptor, ApiError> {
    ensure_running_with_timeout(context, executable, clock, ENSURE_WAIT).await
}

async fn ensure_running_with_timeout(
    context: &RuntimeContext,
    executable: &Path,
    clock: Arc<dyn Clock>,
    timeout: Duration,
) -> Result<EndpointDescriptor, ApiError> {
    let paths = InstancePaths::resolve(context).map_err(io_error)?;
    paths.prepare_instance_dir().map_err(io_error)?;
    let deadline = Instant::now() + timeout;
    let mut launched = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(api_error(
                ErrorCode::HostUnavailable,
                "daemon did not become ready within five seconds; inspect daemon health and logs",
            ));
        }
        match handshake(
            &paths,
            clock.clone(),
            remaining.min(Duration::from_millis(250)),
        )
        .await
        {
            Ok(Some(descriptor)) => return Ok(descriptor),
            Ok(None) => {}
            Err(error) if error.code == ErrorCode::DaemonVersionMismatch => return Err(error),
            Err(error) => return Err(error),
        }
        match OwnerLock::acquire(&paths) {
            Ok(lock) => {
                drop(lock);
                if !launched {
                    spawn_detached(executable, context).map_err(io_error)?;
                    launched = true;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(io_error(error)),
        }
        tokio::time::sleep(
            Duration::from_millis(25).min(deadline.saturating_duration_since(Instant::now())),
        )
        .await;
    }
}

/// The caller supplies readiness and final drain hooks; both execute under the owner lease.
// Allowed: owner entry point: paths, service, clock, shutdown, sink, limits and two lifecycle hooks.
#[allow(clippy::too_many_arguments)]
pub async fn run_owner<S, R, D>(
    paths: &InstancePaths,
    handler: Arc<dyn LocalService>,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
    sink: S,
    limits: BufferLimits,
    on_ready: R,
    on_drained: D,
) -> io::Result<bool>
where
    S: DiagnosticSink,
    R: FnOnce(&EndpointDescriptor) -> io::Result<()>,
    D: FnOnce() -> io::Result<()>,
{
    run_owner_with_factory(
        paths,
        clock,
        shutdown,
        sink,
        limits,
        move |_, _, _| Ok(handler),
        on_ready,
        move || async move { on_drained() },
    )
    .await
}

/// Construct the service only after election and binding reveal the actual boot.
/// The final callback is awaited under the owner lease on every path after factory invocation.
// Allowed: run_owner's inputs with a service factory in place of a built service.
#[allow(clippy::too_many_arguments)]
pub async fn run_owner_with_factory<S, F, R, D, Fut>(
    paths: &InstancePaths,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
    sink: S,
    limits: BufferLimits,
    make_service: F,
    on_ready: R,
    on_drained: D,
) -> io::Result<bool>
where
    S: DiagnosticSink,
    F: FnOnce(Uuid, Uuid, Cancellation) -> io::Result<Arc<dyn LocalService>>,
    R: FnOnce(&EndpointDescriptor) -> io::Result<()>,
    D: FnOnce() -> Fut,
    Fut: Future<Output = io::Result<()>>,
{
    let lease = match OwnerLock::acquire(paths) {
        Ok(lease) => lease,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut owner = start_after_election(Some(lease), sink, limits)?.expect("elected owner");
    if let Err(error) = owner.owner_lease().remove_stale_endpoint() {
        record_owner_error(&mut owner, &error);
        return Err(error);
    }
    let bound = owner
        .owner_lease()
        .bind_socket()
        .inspect_err(|error| record_owner_error(&mut owner, error))?;
    let mut bound = Some(bound);
    let handler = match make_service(
        owner.owner_lease().instance_uuid(),
        bound.as_ref().expect("bound listener").boot_id(),
        shutdown.clone(),
    ) {
        Ok(handler) => handler,
        Err(error) => {
            record_owner_error(&mut owner, &error);
            let teardown_result = on_drained().await;
            if let Err(teardown_error) = &teardown_result {
                record_owner_error(&mut owner, teardown_error);
            }
            let cleanup_result = owner
                .owner_lease()
                .remove_unpublished_bound_socket(bound.as_ref().expect("bound listener"));
            if let Err(cleanup_error) = &cleanup_result {
                record_owner_error(&mut owner, cleanup_error);
            }
            let close_result = owner.drain_and_close();
            cleanup_result?;
            teardown_result?;
            close_result?;
            return Err(error);
        }
    };
    let descriptor_result = owner.owner_lease().publish_endpoint(
        bound.as_ref().expect("bound listener"),
        env!("CARGO_PKG_VERSION"),
        PROTOCOL_VERSION,
    );
    let mut published = None;
    let run_result = match descriptor_result {
        Ok(descriptor) => {
            published = Some(descriptor.clone());
            if let Err(error) = on_ready(&descriptor) {
                Err(error)
            } else {
                let listener = bound.take().expect("bound listener").into_async();
                match listener {
                    Ok(listener) => {
                        let serve_result = transport::serve(
                            listener,
                            descriptor.instance_uuid,
                            handler.clone(),
                            clock,
                            effective_uid(),
                            shutdown,
                        )
                        .await;
                        match serve_result {
                            Ok(ServeOutcome::Drained) => Ok(()),
                            Ok(ServeOutcome::Incomplete(mut pending)) => pending.wait().await,
                            Err(error) => Err(error),
                        }
                    }
                    Err(error) => Err(error),
                }
            }
        }
        Err(error) => Err(error),
    };
    // Retain the service and lock while its own background work is joined.
    let teardown_result = on_drained().await;
    drop(handler);
    if let Err(error) = &run_result {
        record_owner_error(&mut owner, error);
    }
    if let Err(error) = &teardown_result {
        record_owner_error(&mut owner, error);
    }
    let remove_result = match published {
        Some(descriptor) => owner
            .owner_lease()
            .remove_owned_endpoint(descriptor.boot_id),
        None => owner
            .owner_lease()
            .remove_failed_bound_publication(bound.as_ref().expect("bound listener")),
    };
    if let Err(error) = &remove_result {
        record_owner_error(&mut owner, error);
    }
    owner.stop_admission();
    let close_result = owner.drain_and_close();
    remove_result?;
    teardown_result?;
    run_result?;
    close_result?;
    Ok(true)
}

#[cfg(test)]
#[path = "../../tests/daemon/lifecycle.rs"]
mod tests;
