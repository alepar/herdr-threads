//! Shutdown and detached child output controls.

use crate::daemon::diagnostics::DiagnosticSource;
use crate::daemon::{diagnostics::DiagnosticSink, lifecycle::OwnerSession};
use crate::daemon::{
    health::HealthInputs,
    ownership::{
        EndpointDescriptor, owner_lock_identity, previous_owner_released, read_descriptor,
    },
    paths::InstancePaths,
};
use crate::ports::{LocalClient, LocalService, ServiceAuthorityGate, ServiceConnectionAuthority};
use crate::protocol::{
    authority::PeerIdentity,
    commands::{Command as ApiCommand, StopRequest},
    output::OutputSpec,
    results::{
        ApiError, CapabilityList, CommandResult, ErrorCode, ServiceDisconnectResult,
        ServiceRecoveryAudit, StopAccepted,
    },
    service::{ServiceOperation, ServiceResult},
    time::{CallBudget, Cancellation, Clock},
    wire::PROTOCOL_VERSION,
};
use crate::service::live_gate::LiveServiceGate;
use std::{
    io::{self, Read},
    process::{Command, ExitStatus, Stdio},
    sync::mpsc::{self, SyncSender},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

/// The service invokes this only after the transport has checked the wire
/// instance. Rechecking both identities here fences an old boot before it can
/// cancel the current owner. Cancellation is observed by lifecycle admission.
pub struct StopController {
    instance: Uuid,
    boot: Uuid,
    shutdown: Cancellation,
}

impl StopController {
    pub fn new(instance: Uuid, boot: Uuid, shutdown: Cancellation) -> Self {
        Self {
            instance,
            boot,
            shutdown,
        }
    }

    pub fn request_stop(&self, instance: Uuid, boot: Uuid) -> Result<(), ApiError> {
        if instance != self.instance || boot != self.boot {
            return Err(ApiError::instance_mismatch(
                "stop target is not this daemon boot",
            ));
        }
        self.shutdown.cancel();
        Ok(())
    }
}

/// Service control is kept outside the domain command adapter. Transport
/// verifies WireRequest.expected_instance before invoking this handler.
pub struct ControlService<H, S> {
    stop: StopController,
    health: H,
    domain: S,
    hook_parse_failures: Option<std::sync::Arc<crate::daemon::logs::HookParseFailures>>,
    /// The harness version manifest (ht-xoc.4 calls `ensure_manifest` from
    /// its evidence handler; ht-xoc.5 reads `current()`). Not called yet.
    #[allow(dead_code)]
    harness_manifest: Option<std::sync::Arc<crate::harness::manifest::ManifestService>>,
}

impl<H, S> ControlService<H, S>
where
    H: Fn(&CallBudget) -> HealthInputs + Send + Sync,
    S: LocalService,
{
    pub fn new(stop: StopController, health: H, domain: S) -> Self {
        Self {
            stop,
            health,
            domain,
            hook_parse_failures: None,
            harness_manifest: None,
        }
    }

    /// Where hook parse-failure reports are counted and logged. Without it
    /// the report is accepted and dropped.
    pub fn with_hook_parse_failures(
        mut self,
        failures: std::sync::Arc<crate::daemon::logs::HookParseFailures>,
    ) -> Self {
        self.hook_parse_failures = Some(failures);
        self
    }

    /// The daemon's harness version manifest service.
    pub fn with_harness_manifest(
        mut self,
        manifest: std::sync::Arc<crate::harness::manifest::ManifestService>,
    ) -> Self {
        self.harness_manifest = Some(manifest);
        self
    }
}

impl<H, S> LocalService for ControlService<H, S>
where
    H: Fn(&CallBudget) -> HealthInputs + Send + Sync,
    S: LocalService,
{
    fn service_control(
        &self,
        command: ApiCommand,
        peer: PeerIdentity,
        instance: &str,
        boot: &str,
        gate: &LiveServiceGate,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if peer.effective_uid() != crate::daemon::paths::effective_uid()
            || instance != self.stop.instance.to_string()
            || boot != self.stop.boot.to_string()
        {
            return Err(ApiError::unauthorized(
                "service recovery requires this daemon's local owner",
            ));
        }
        match command {
            ApiCommand::ServiceInspect => Ok(CommandResult::ServiceInspection(
                gate.inspect(instance, boot),
            )),
            ApiCommand::ServiceDisconnect(request) => {
                if request.expected_boot != boot
                    || !gate.disconnect(instance, boot, request.expected_generation)
                {
                    return Err(ApiError::stale_service_generation(
                        "observed service boot or generation is no longer active",
                    ));
                }
                let audit = match self.domain.audit_service_disconnect(
                    boot,
                    request.expected_generation,
                    peer,
                    budget,
                ) {
                    Ok(()) => ServiceRecoveryAudit::Persisted,
                    Err(error) => ServiceRecoveryAudit::Failed(error),
                };
                Ok(CommandResult::ServiceDisconnected(
                    ServiceDisconnectResult {
                        instance: instance.into(),
                        daemon_boot: boot.into(),
                        connection_generation: request.expected_generation,
                        disconnected: true,
                        audit,
                    },
                ))
            }
            _ => Err(ApiError::invalid_request(
                "invalid service recovery command",
            )),
        }
    }

    fn audit_service_disconnect(
        &self,
        boot: &str,
        generation: u64,
        peer: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        self.domain
            .audit_service_disconnect(boot, generation, peer, budget)
    }

    fn service_operation(
        &self,
        operation: ServiceOperation,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        budget: &CallBudget,
    ) -> Result<ServiceResult, ApiError> {
        self.domain
            .service_operation(operation, connection, gate, budget)
    }
    fn handle(
        &self,
        command: ApiCommand,
        peer: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.handle_with_output(command, peer, budget, &OutputSpec::default())
    }

    fn handle_with_output(
        &self,
        command: ApiCommand,
        peer: PeerIdentity,
        budget: &CallBudget,
        output: &OutputSpec,
    ) -> Result<CommandResult, ApiError> {
        match command {
            // Health reads storage under this request's cancellation and deadline.
            ApiCommand::Health => Ok(CommandResult::Health((self.health)(budget).assemble())),
            ApiCommand::Capabilities => Ok(CommandResult::Capabilities(CapabilityList {
                capabilities: crate::protocol::capabilities::ADVERTISED
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
            })),
            ApiCommand::HookParseFailure(report) => {
                if let Some(failures) = &self.hook_parse_failures {
                    failures.record(&report.harness, &report.detail);
                }
                Ok(CommandResult::HookParseFailureRecorded)
            }
            ApiCommand::Stop(StopRequest { expected_boot }) => {
                let boot = Uuid::parse_str(&expected_boot)
                    .map_err(|_| ApiError::invalid_request("invalid stop boot ID"))?;
                self.stop.request_stop(self.stop.instance, boot)?;
                Ok(CommandResult::StopAccepted(StopAccepted {
                    boot_id: self.stop.boot.to_string(),
                }))
            }
            command => self
                .domain
                .handle_with_output(command, peer, budget, output),
        }
    }
}

fn stop_error(code: ErrorCode, detail: impl Into<String>) -> ApiError {
    ApiError::new(code, detail)
}

/// Bounded wait for a SIGTERMed skewed owner to release its lock.
const SKEW_STOP_WAIT: Duration = Duration::from_secs(10);

/// What "the pid and boot_id match the running daemon" means, mechanically
/// (the boot_id is published nowhere else): the owner lock is held; the
/// descriptor re-read after that probe is identical (same pid and boot_id);
/// the pid is a live process other than this one; its command line is a
/// `daemon run` for this state directory (`ps -o command= -p <pid>`); and the
/// socket at `descriptor.endpoint` is the one this boot bound (the
/// descriptor's `socket_device`/`socket_inode`). Any mismatch refuses and no
/// signal is sent. `Ok(false)` means no owner holds the lock: nothing to stop.
fn confirm_skewed_owner(
    paths: &InstancePaths,
    descriptor: &EndpointDescriptor,
) -> Result<bool, ApiError> {
    use std::os::unix::fs::MetadataExt;
    let refuse = |why: &str| {
        Err(stop_error(
            ErrorCode::HostUnavailable,
            format!(
                "refusing to signal pid {}: {why}; nothing was stopped",
                descriptor.pid
            ),
        ))
    };
    let io_failure = |error: io::Error| {
        stop_error(
            ErrorCode::HostUnavailable,
            format!("unable to identify daemon owner: {error}"),
        )
    };
    let lock = owner_lock_identity(paths).map_err(io_failure)?;
    if previous_owner_released(paths, lock).map_err(io_failure)? {
        return Ok(false);
    }
    match read_descriptor(paths, descriptor.instance_uuid) {
        Ok(current) if &current == descriptor => {}
        Ok(_) => return refuse("the published descriptor changed (pid or boot_id differs)"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return refuse("the endpoint descriptor vanished");
        }
        Err(error) => return Err(io_failure(error)),
    }
    if descriptor.pid <= 1 || descriptor.pid == std::process::id() {
        return refuse("the descriptor names an unsignalable pid");
    }
    // SAFETY: signal 0 only probes for existence.
    if unsafe { libc::kill(descriptor.pid as libc::pid_t, 0) } != 0 {
        return refuse("the descriptor's pid is not a live process we may signal");
    }
    let command = Command::new("ps")
        .args(["-ww", "-o", "command=", "-p"])
        .arg(descriptor.pid.to_string())
        .output()
        .map_err(io_failure)?;
    let command = String::from_utf8_lossy(&command.stdout).into_owned();
    let state = paths
        .instance_dir
        .parent()
        .and_then(std::path::Path::parent)
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let words: Vec<&str> = command.split_whitespace().collect();
    let runs_daemon = words.windows(2).any(|pair| pair == ["daemon", "run"]);
    let this_state = words
        .windows(2)
        .any(|pair| pair[0] == "--state-dir" && pair[1] == state);
    if !(runs_daemon && this_state) {
        return refuse("its command line is not a `daemon run` for this state directory");
    }
    match std::fs::symlink_metadata(&descriptor.endpoint) {
        Ok(meta)
            if meta.dev() == descriptor.socket_device && meta.ino() == descriptor.socket_inode => {}
        _ => return refuse("the socket is not the one this boot bound (boot_id mismatch)"),
    }
    Ok(true)
}

/// Request shutdown of the daemon `descriptor` describes. The supplied client
/// must be bound to the descriptor endpoint, instance and boot. With a
/// matching protocol the wire Stop is sent and a successful response
/// acknowledges shutdown. With a mismatched protocol (root §B3 D3 as amended
/// r2) nothing is decoded and no wire Stop is sent: the owner is confirmed
/// pid/lock based (see `confirm_skewed_owner`) and SIGTERMed. Lifecycle drain
/// follows either way.
pub fn request_stop(
    client: &dyn LocalClient,
    paths: &InstancePaths,
    descriptor: &EndpointDescriptor,
    budget: &CallBudget,
) -> Result<(), ApiError> {
    if descriptor.protocol_version != PROTOCOL_VERSION {
        if confirm_skewed_owner(paths, descriptor)? {
            // SAFETY: the pid was confirmed above to be this state's daemon.
            if unsafe { libc::kill(descriptor.pid as libc::pid_t, libc::SIGTERM) } != 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(stop_error(
                        ErrorCode::HostUnavailable,
                        format!("unable to signal pid {}: {error}", descriptor.pid),
                    ));
                }
            }
        }
        return Ok(());
    }
    let result = client.call(
        ApiCommand::Stop(StopRequest {
            expected_boot: descriptor.boot_id.to_string(),
        }),
        budget,
    )?;
    match result {
        CommandResult::StopAccepted(accepted)
            if accepted.boot_id == descriptor.boot_id.to_string() =>
        {
            Ok(())
        }
        _ => Err(stop_error(
            ErrorCode::UnknownOutcome,
            "stop response did not confirm requested boot",
        )),
    }
}

/// Request shutdown, then wait for the prior owner's lock lease to release.
/// The deadline includes both the IPC exchange and the owner drain; a
/// skew stop waits up to ten seconds for the SIGTERMed owner regardless.
pub fn stop_and_wait(
    client: &dyn LocalClient,
    paths: &InstancePaths,
    descriptor: &EndpointDescriptor,
    clock: &dyn Clock,
    budget: &CallBudget,
) -> Result<(), ApiError> {
    let prior_lock = owner_lock_identity(paths).map_err(|error| {
        ApiError::host_unavailable(format!(
            "unable to identify daemon owner lock: {error}; inspect daemon health and owner lock"
        ))
    })?;
    request_stop(client, paths, descriptor, budget)?;
    let remaining_ms = budget.deadline.0.saturating_sub(clock.monotonic_now().0);
    let wait = if descriptor.protocol_version == PROTOCOL_VERSION {
        Duration::from_millis(remaining_ms).min(Duration::from_secs(5))
    } else {
        SKEW_STOP_WAIT
    };
    wait_for_stop_with_probe(wait, &budget.cancellation, || {
        match read_descriptor(paths, descriptor.instance_uuid) {
            Ok(current) if current.boot_id != descriptor.boot_id => Ok(true),
            Ok(_) => previous_owner_released(paths, prior_lock),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                previous_owner_released(paths, prior_lock)
            }
            Err(error) => Err(error),
        }
    })
}

fn wait_for_stop_with_probe(
    remaining: Duration,
    cancellation: &Cancellation,
    mut stopped: impl FnMut() -> io::Result<bool>,
) -> Result<(), ApiError> {
    let deadline = Instant::now() + remaining;
    loop {
        if cancellation.is_cancelled() {
            return Err(ApiError::cancelled("daemon stop wait cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(ApiError::deadline_exceeded(
                "daemon accepted stop but did not exit before the deadline; inspect its health and owner lock",
            ));
        }
        if stopped().map_err(|error| {
            ApiError::host_unavailable(format!(
                "unable to verify daemon stop: {error}; inspect daemon health and owner lock"
            ))
        })? {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ApiError::deadline_exceeded(
                "daemon accepted stop but did not exit before the deadline; inspect its health and owner lock",
            ));
        }
        let pause = remaining.min(Duration::from_millis(10));
        thread::sleep(pause);
    }
}

enum StreamPacket {
    Bytes(DiagnosticSource, Vec<u8>),
    Error(io::Error),
}

fn read_stream<R: Read>(
    mut stream: R,
    source: DiagnosticSource,
    max_fragment: usize,
    sender: SyncSender<StreamPacket>,
) {
    let mut fragment = vec![0; max_fragment];
    loop {
        match stream.read(&mut fragment) {
            Ok(0) => break,
            Ok(n) => {
                if sender
                    .send(StreamPacket::Bytes(source, fragment[..n].to_vec()))
                    .is_err()
                {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let _ = sender.send(StreamPacket::Error(error));
                break;
            }
        }
    }
}

/// Capture both streams into the elected owner's bounded sink while the child runs.
/// Reader threads and their channel are owned here; no ensure caller drains them.
pub fn run_child_with_diagnostics<S: DiagnosticSink>(
    owner: &mut OwnerSession<S>,
    command: &mut Command,
) -> io::Result<ExitStatus> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing child stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing child stderr"))?;
    let (sender, receiver) = mpsc::sync_channel(2);
    let max_fragment = owner.max_fragment_bytes();
    let stdout_sender = sender.clone();
    let stdout_reader = thread::spawn(move || {
        read_stream(
            stdout,
            DiagnosticSource::Stdout,
            max_fragment,
            stdout_sender,
        )
    });
    let stderr_reader =
        thread::spawn(move || read_stream(stderr, DiagnosticSource::Stderr, max_fragment, sender));
    let mut first_error = None;
    for packet in receiver {
        match packet {
            StreamPacket::Bytes(source, fragment) if first_error.is_none() => {
                if let Err(error) = owner.emit(source, &fragment).and_then(|_| owner.flush()) {
                    first_error = Some(error);
                }
            }
            StreamPacket::Bytes(_, fragment) => owner.discard_bytes(fragment.len()),
            StreamPacket::Error(error) if first_error.is_none() => first_error = Some(error),
            StreamPacket::Error(_) => {}
        }
    }
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    let status = child.wait()?;
    if let Some(error) = first_error {
        Err(error)
    } else {
        Ok(status)
    }
}

/// Call after transport admission has stopped and active requests have drained.
/// An owned listener independently retains the lock until the transport drops it.
pub fn drain_and_close<S: DiagnosticSink>(mut owner: OwnerSession<S>) -> io::Result<()> {
    owner.drain_and_close()
}

#[cfg(test)]
#[path = "../../tests/daemon/control.rs"]
mod tests;
