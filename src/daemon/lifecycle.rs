//! Owner-scoped child diagnostics lifecycle.

use crate::client::local::LocalSocketClient;
use crate::daemon::diagnostics::{
    BufferLimits, DiagnosticBuffer, DiagnosticSink, DiagnosticSource,
};
use crate::daemon::logs::{
    StartAttempt, create_startup_log, daemon_log_path, prune_startup_logs, read_startup_tail,
    startup_log_path, startup_logs_dir, startup_tail_text,
};
use crate::daemon::ownership::{
    EndpointDescriptor, OwnerLock, owner_lock_identity, previous_owner_released, read_descriptor,
    read_existing_namespace,
};
use crate::daemon::paths::{InstancePaths, RuntimeContext, effective_uid};
use crate::daemon::remedy::{RemedyContext, remedy};
use crate::daemon::transport::{self, ServeOutcome};
use crate::ports::LocalService;
use crate::protocol::{
    commands::Command,
    results::{ApiError, CommandResult, ErrorClass, ErrorCode, HealthState},
    time::{CallBudget, Cancellation, Clock, MonoInstant},
    wire::PROTOCOL_VERSION,
};
use std::{
    future::Future,
    io::{self, Read},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant, SystemTime},
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
    ApiError::new(code, detail)
}

/// Refuse a published daemon whose protocol differs from this executable's
/// before any request is sent: an older daemon drops a newer request at
/// decode with no reply, so a caller would otherwise wait out its budget.
/// The descriptor-only check the native hook applies (it has no time to
/// confirm the owner is alive); the CLI and `ensure` use [`live_skew`], which
/// also tells a stale descriptor from a live skewed daemon. The text is the
/// one VersionSkew `remedy()` line; `daemon stop` from this executable is
/// skew-tolerant, so the remedy works from the CLI that printed it.
pub(crate) fn check_protocol(descriptor: &EndpointDescriptor) -> Result<(), ApiError> {
    if descriptor.protocol_version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(skew_error(
            ErrorCode::UnknownWireVersion,
            &descriptor.software_version,
            descriptor.protocol_version,
        ))
    }
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

/// The VersionSkew error: `code` stays the caller's (wire vs software skew),
/// the text is the one `remedy()` line naming both versions.
pub(crate) fn skew_error(code: ErrorCode, daemon_software: &str, daemon_protocol: u16) -> ApiError {
    api_error(
        code,
        remedy(
            Some(ErrorClass::VersionSkew),
            &RemedyContext::VersionSkew {
                daemon: format!("{daemon_software} (protocol {daemon_protocol})"),
                cli: format!(
                    "{} (protocol {PROTOCOL_VERSION})",
                    env!("CARGO_PKG_VERSION")
                ),
            },
        ),
    )
}

/// Skew error for a descriptor whose protocol this CLI does not speak, when
/// its owner is still alive. `None` when the owner is gone (stale descriptor,
/// nothing to skew against). Never decodes anything the daemon wrote.
pub(crate) fn live_skew(
    paths: &InstancePaths,
    instance: Uuid,
    descriptor: &EndpointDescriptor,
) -> Result<Option<ApiError>, ApiError> {
    let lock_identity = owner_lock_identity(paths).map_err(io_error)?;
    if previous_owner_released(paths, lock_identity).map_err(io_error)? {
        return Ok(None);
    }
    let current = match read_descriptor(paths, instance) {
        Ok(current) => current,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    if &current != descriptor || previous_owner_released(paths, lock_identity).map_err(io_error)? {
        return Ok(None);
    }
    Ok(Some(skew_error(
        ErrorCode::UnknownWireVersion,
        &descriptor.software_version,
        descriptor.protocol_version,
    )))
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
        // Skew is read from the descriptor before any wire decode.
        return match live_skew(paths, instance, &descriptor)? {
            Some(error) => Err(error),
            None => Ok(None),
        };
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
        return Err(skew_error(
            ErrorCode::UnknownWireVersion,
            &health.software_version,
            health.protocol_version,
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
        return Err(skew_error(
            ErrorCode::DaemonVersionMismatch,
            &health.software_version,
            health.protocol_version,
        ));
    }
    // A reachable, identity-matching daemon that reports Degraded (for
    // example unsupported native capabilities) is running and serving; the
    // caller prints its health. Only Unavailable is a startup failure.
    if health.state == HealthState::Unavailable {
        return Err(api_error(
            ErrorCode::HostUnavailable,
            format!(
                "daemon is reachable but unavailable; {}",
                remedy(
                    Some(ErrorClass::Unavailable),
                    &RemedyContext::StartupFailure {
                        log: daemon_log_path(paths)
                    }
                )
            ),
        ));
    }
    Ok(Some(descriptor))
}

/// Where one detached child's stderr goes, kept by the starter that spawned it
/// so it can print only its own attempt's output.
enum StartupStderr {
    /// The per-attempt startup log the child writes through its descriptor.
    File(PathBuf),
    /// `<state>/logs` could not be used: the child's stderr is a bounded pipe
    /// whose last bytes this starter keeps.
    Piped {
        reason: String,
        tail: Arc<Mutex<Vec<u8>>>,
        done: ReaderDone,
    },
}

/// Set (and signalled) when the stderr reader thread reaches end of file.
type ReaderDone = Arc<(Mutex<bool>, Condvar)>;

/// A spawned detached child and its stderr destination.
struct Launched {
    child: Child,
    stderr: StartupStderr,
}

/// Bytes of the child's stderr kept in the fallback ring.
const FALLBACK_TAIL_BYTES: usize = 8 * 1024;

fn pipe_reader(mut pipe: impl Read + Send + 'static) -> (Arc<Mutex<Vec<u8>>>, ReaderDone) {
    let tail = Arc::new(Mutex::new(Vec::new()));
    let done = Arc::new((Mutex::new(false), Condvar::new()));
    let (ring, finished) = (Arc::clone(&tail), Arc::clone(&done));
    let _ = std::thread::Builder::new()
        .name("child-stderr-tail".into())
        .spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(size) = pipe.read(&mut chunk) {
                if size == 0 {
                    break;
                }
                if let Ok(mut ring) = ring.lock() {
                    ring.extend_from_slice(&chunk[..size]);
                    let excess = ring.len().saturating_sub(FALLBACK_TAIL_BYTES);
                    ring.drain(..excess);
                }
            }
            if let Ok(mut flag) = finished.0.lock() {
                *flag = true;
                finished.1.notify_all();
            }
        });
    (tail, done)
}

fn spawn_detached(
    executable: &Path,
    context: &RuntimeContext,
    paths: &InstancePaths,
    attempt: &StartAttempt,
) -> io::Result<Launched> {
    let mut command = ProcessCommand::new(executable);
    command
        .arg("daemon")
        .arg("run")
        .arg("--state-dir")
        .arg(&context.state_dir)
        .arg("--host-endpoint")
        .arg(&context.host_endpoint)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    let file_log = create_startup_log(paths, attempt);
    let fallback_reason = match file_log {
        Ok((file, path)) => {
            prune_startup_logs(&startup_logs_dir(paths), &path, SystemTime::now());
            command.stderr(Stdio::from(file));
            Ok(path)
        }
        Err(error) => {
            command.stderr(Stdio::piped());
            Err(error.to_string())
        }
    };
    if let Some(path) = &context.herdr_bin {
        command.env("HERDR_BIN_PATH", path);
    } else {
        command.env_remove("HERDR_BIN_PATH");
    }
    command.process_group(0);
    let mut child = command.spawn()?;
    let stderr = match fallback_reason {
        Ok(path) => StartupStderr::File(path),
        Err(reason) => {
            let pipe = child.stderr.take().expect("piped stderr");
            let (tail, done) = pipe_reader(pipe);
            StartupStderr::Piped { reason, tail, done }
        }
    };
    Ok(Launched { child, stderr })
}

impl Launched {
    /// The operator-facing block for this attempt only: its startup log's
    /// tail naming the path, or the fallback's pipe tail naming the fallback.
    /// Empty when the child wrote nothing.
    fn tail_block(&self) -> String {
        match &self.stderr {
            StartupStderr::File(path) => {
                let tail = read_startup_tail(path).unwrap_or_default();
                if tail.is_empty() {
                    String::new()
                } else {
                    format!("startup log {}:\n{tail}", path.display())
                }
            }
            StartupStderr::Piped { reason, tail, done } => {
                // The child has exited or timed out; give the reader a moment
                // to drain what is already in the pipe.
                if let Ok(flag) = done.0.lock() {
                    let _ =
                        done.1
                            .wait_timeout_while(flag, Duration::from_millis(200), |finished| {
                                !*finished
                            });
                }
                let bytes = tail.lock().map(|ring| ring.clone()).unwrap_or_default();
                format!(
                    "(startup log unavailable: {reason}; showing the child's stderr)\n{}",
                    startup_tail_text(&bytes)
                )
            }
        }
    }

    /// The log file the remedy names: the attempt's file, or `daemon.log`
    /// when the attempt had no file.
    fn remedy_log(&self, paths: &InstancePaths) -> PathBuf {
        match &self.stderr {
            StartupStderr::File(path) => path.clone(),
            StartupStderr::Piped { .. } => daemon_log_path(paths),
        }
    }
}

/// One error text: the summary, this attempt's output block when it has any,
/// then the remedy. Without output the remedy follows the summary inline.
fn startup_failure_text(summary: &str, block: &str, remedy: &str) -> String {
    if block.is_empty() {
        format!("{summary}; {remedy}")
    } else {
        format!("{summary}\n{block}\n{remedy}")
    }
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
    let attempt = StartAttempt::new();
    let mut launched: Option<Launched> = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let log = launched.as_ref().map_or_else(
                || startup_log_path(&paths, &attempt),
                |launched| launched.remedy_log(&paths),
            );
            let remedy = remedy(
                Some(ErrorClass::Unavailable),
                &RemedyContext::StartupTimeout { log },
            );
            let block = launched
                .as_ref()
                .map(Launched::tail_block)
                .unwrap_or_default();
            return Err(api_error(
                ErrorCode::HostUnavailable,
                startup_failure_text(
                    "daemon did not become ready within five seconds",
                    &block,
                    &remedy,
                ),
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
        // A child that exited with a failure before becoming ready will never
        // be ready: report its own output now instead of waiting out the
        // deadline. A clean exit is a lost election; the winner's readiness is
        // what the loop is waiting for.
        if let Some(launched) = launched.as_mut()
            && let Ok(Some(status)) = launched.child.try_wait()
            && !status.success()
        {
            let remedy = remedy(
                None,
                &RemedyContext::StartupFailure {
                    log: launched.remedy_log(&paths),
                },
            );
            return Err(api_error(
                ErrorCode::HostUnavailable,
                startup_failure_text(
                    &format!("daemon exited during startup ({status})"),
                    &launched.tail_block(),
                    &remedy,
                ),
            ));
        }
        match OwnerLock::acquire(&paths) {
            Ok(lock) => {
                drop(lock);
                if launched.is_none() {
                    launched = Some(
                        spawn_detached(executable, context, &paths, &attempt).map_err(io_error)?,
                    );
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

#[cfg(test)]
#[path = "../../tests/daemon/startup_log.rs"]
mod startup_log_tests;
