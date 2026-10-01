//! Shutdown and detached child output controls.

use crate::daemon::diagnostics::DiagnosticSource;
use crate::daemon::{diagnostics::DiagnosticSink, lifecycle::OwnerSession};
use crate::daemon::{
    health::HealthInputs,
    ownership::{
        EndpointDescriptor, owner_lock_identity, previous_owner_released, read_descriptor,
    },
    paths::InstancePaths,
    transport::LiveServiceGate,
};
use crate::ports::{LocalClient, LocalService, ServiceAuthorityGate, ServiceConnectionAuthority};
use crate::protocol::{
    authority::PeerIdentity,
    commands::{Command as ApiCommand, StopRequest},
    output::OutputSpec,
    results::{
        ApiError, CommandResult, ErrorCode, ServiceDisconnectResult, ServiceRecoveryAudit,
        StopAccepted,
    },
    service::{ServiceOperation, ServiceResult},
    time::{CallBudget, Cancellation, Clock},
    wire::PROTOCOL_VERSION,
};
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
            return Err(ApiError {
                code: ErrorCode::InstanceMismatch,
                detail: "stop target is not this daemon boot".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            });
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
        }
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
            return Err(ApiError {
                code: ErrorCode::Unauthorized,
                detail: "service recovery requires this daemon's local owner".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            });
        }
        match command {
            ApiCommand::ServiceInspect => Ok(CommandResult::ServiceInspection(
                gate.inspect(instance, boot),
            )),
            ApiCommand::ServiceDisconnect(request) => {
                if request.expected_boot != boot
                    || !gate.disconnect(instance, boot, request.expected_generation)
                {
                    return Err(ApiError {
                        code: ErrorCode::StaleServiceGeneration,
                        detail: "observed service boot or generation is no longer active".into(),
                        restart_argv: None,
                        required_minimum_bytes: None,
                    });
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
            _ => Err(ApiError {
                code: ErrorCode::InvalidRequest,
                detail: "invalid service recovery command".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            }),
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
            ApiCommand::Stop(StopRequest { expected_boot }) => {
                let boot = Uuid::parse_str(&expected_boot).map_err(|_| ApiError {
                    code: ErrorCode::InvalidRequest,
                    detail: "invalid stop boot ID".into(),
                    restart_argv: None,
                    required_minimum_bytes: None,
                })?;
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

/// The supplied client must be bound to the descriptor endpoint, instance and
/// boot. A successful response acknowledges shutdown; lifecycle drain follows.
pub fn request_stop(
    client: &dyn LocalClient,
    descriptor: &EndpointDescriptor,
    budget: &CallBudget,
) -> Result<(), ApiError> {
    if descriptor.protocol_version != PROTOCOL_VERSION {
        return Err(ApiError {
            code: ErrorCode::UnknownWireVersion,
            detail: "daemon protocol is incompatible with stop".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        });
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
        _ => Err(ApiError {
            code: ErrorCode::UnknownOutcome,
            detail: "stop response did not confirm requested boot".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }),
    }
}

/// Request shutdown, then wait for the prior owner's lock lease to release.
/// The deadline includes both the IPC exchange and the owner drain.
pub fn stop_and_wait(
    client: &dyn LocalClient,
    paths: &InstancePaths,
    descriptor: &EndpointDescriptor,
    clock: &dyn Clock,
    budget: &CallBudget,
) -> Result<(), ApiError> {
    let prior_lock = owner_lock_identity(paths).map_err(|error| ApiError {
        code: ErrorCode::HostUnavailable,
        detail: format!(
            "unable to identify daemon owner lock: {error}; inspect daemon health and owner lock"
        ),
        restart_argv: None,
        required_minimum_bytes: None,
    })?;
    request_stop(client, descriptor, budget)?;
    let remaining_ms = budget.deadline.0.saturating_sub(clock.monotonic_now().0);
    wait_for_stop_with_probe(
        Duration::from_millis(remaining_ms).min(Duration::from_secs(5)),
        &budget.cancellation,
        || match read_descriptor(paths, descriptor.instance_uuid) {
            Ok(current) if current.boot_id != descriptor.boot_id => Ok(true),
            Ok(_) => previous_owner_released(paths, prior_lock),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                previous_owner_released(paths, prior_lock)
            }
            Err(error) => Err(error),
        },
    )
}

fn wait_for_stop_with_probe(
    remaining: Duration,
    cancellation: &Cancellation,
    mut stopped: impl FnMut() -> io::Result<bool>,
) -> Result<(), ApiError> {
    let deadline = Instant::now() + remaining;
    loop {
        if cancellation.is_cancelled() {
            return Err(ApiError {
                code: ErrorCode::Cancelled,
                detail: "daemon stop wait cancelled".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            });
        }
        if Instant::now() >= deadline {
            return Err(ApiError {
                code: ErrorCode::DeadlineExceeded,
                detail: "daemon accepted stop but did not exit before the deadline; inspect its health and owner lock".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            });
        }
        if stopped().map_err(|error| ApiError {
            code: ErrorCode::HostUnavailable,
            detail: format!(
                "unable to verify daemon stop: {error}; inspect daemon health and owner lock"
            ),
            restart_argv: None,
            required_minimum_bytes: None,
        })? {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ApiError {
                code: ErrorCode::DeadlineExceeded,
                detail: "daemon accepted stop but did not exit before the deadline; inspect its health and owner lock".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            });
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
