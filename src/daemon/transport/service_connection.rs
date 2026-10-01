//! Live authority for the one registered programmatic connection.

use super::*;
use crate::{
    ports::{
        ServiceAuthorityGate, ServiceConnectionAuthority, ServiceDecisionGuard,
        ServiceWriteTransactionProof,
    },
    protocol::{
        ids::ServiceAuthorId,
        results::ServiceConnectionInspection,
        service::{
            ServiceRegister, ServiceRegistration, ServiceRequest, ServiceResult,
            ServiceWireRequest, ServiceWireResponse,
        },
        time::UtcMillis,
    },
};
use std::sync::{Mutex, MutexGuard};

struct RegisteredSession {
    connection: Arc<ServiceConnectionAuthority>,
    registered_at: UtcMillis,
    cancellation: Cancellation,
}

struct LiveState {
    next_generation: u64,
    active: Option<RegisteredSession>,
    last_registered_at: Option<UtcMillis>,
}

/// The mutex orders revocation against a store decision guard. No database or
/// external call is made while registering or revoking.
pub struct LiveServiceGate(Mutex<LiveState>);

impl LiveServiceGate {
    pub fn new() -> Self {
        Self(Mutex::new(LiveState {
            next_generation: 0,
            active: None,
            last_registered_at: None,
        }))
    }

    fn state(&self) -> MutexGuard<'_, LiveState> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn register_session(
        &self,
        instance: &str,
        boot: &str,
        author: ServiceAuthorId,
        registered_at: UtcMillis,
    ) -> Result<(Arc<ServiceConnectionAuthority>, Cancellation), ApiError> {
        let mut state = self
            .0
            .try_lock()
            .map_err(|_| service_error(ErrorCode::ServiceBusy, "service authority is deciding"))?;
        if state.active.is_some() {
            return Err(service_error(
                ErrorCode::ServiceBusy,
                "a service connection is already registered",
            ));
        }
        state.next_generation = state
            .next_generation
            .checked_add(1)
            .ok_or_else(|| service_error(ErrorCode::ServiceBusy, "service generation exhausted"))?;
        let connection = Arc::new(ServiceConnectionAuthority::new(
            instance.into(),
            boot.into(),
            state.next_generation,
            author,
        ));
        let cancellation = Cancellation::default();
        state.last_registered_at = Some(registered_at);
        state.active = Some(RegisteredSession {
            connection: connection.clone(),
            registered_at,
            cancellation: cancellation.clone(),
        });
        Ok((connection, cancellation))
    }

    pub fn inspect(&self, instance: &str, boot: &str) -> ServiceConnectionInspection {
        let state = self.state();
        ServiceConnectionInspection {
            instance: instance.into(),
            daemon_boot: boot.into(),
            connected: state.active.is_some(),
            connection_generation: state.active.as_ref().map_or_else(
                || (state.next_generation != 0).then_some(state.next_generation),
                |session| Some(session.connection.generation()),
            ),
            registered_at: state
                .active
                .as_ref()
                .map_or(state.last_registered_at, |session| {
                    Some(session.registered_at)
                }),
        }
    }

    pub fn disconnect(&self, instance: &str, boot: &str, generation: u64) -> bool {
        let mut state = self.state();
        if !state.active.as_ref().is_some_and(|session| {
            session.connection.instance() == instance
                && session.connection.boot() == boot
                && session.connection.generation() == generation
        }) {
            return false;
        }
        let session = state.active.take().expect("matching service session");
        session.cancellation.cancel();
        true
    }
}

fn service_error(code: ErrorCode, detail: &str) -> ApiError {
    api_error(code, detail)
}

struct DecisionGuard<'a> {
    _state: MutexGuard<'a, LiveState>,
    author: ServiceAuthorId,
}

impl ServiceDecisionGuard for DecisionGuard<'_> {
    fn author(&self) -> &ServiceAuthorId {
        &self.author
    }
}

impl ServiceAuthorityGate for LiveServiceGate {
    fn register(
        &self,
        instance: &str,
        boot: &str,
        author: ServiceAuthorId,
    ) -> Result<ServiceConnectionAuthority, ApiError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let (connection, _) = self.register_session(instance, boot, author, UtcMillis(now))?;
        Ok(ServiceConnectionAuthority::new(
            instance.into(),
            boot.into(),
            connection.generation(),
            connection.author().clone(),
        ))
    }

    fn decision_guard<'a>(
        &'a self,
        _: &ServiceWriteTransactionProof,
        connection: &ServiceConnectionAuthority,
    ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
        let state = self.state();
        let matches = state
            .active
            .as_ref()
            .is_some_and(|session| session.connection.as_ref() == connection);
        if !matches {
            return Err(service_error(
                ErrorCode::StaleServiceGeneration,
                "service connection was revoked",
            ));
        }
        Ok(Box::new(DecisionGuard {
            _state: state,
            author: connection.author().clone(),
        }))
    }

    fn revoke_exact(&self, connection: &ServiceConnectionAuthority) -> bool {
        let mut state = self.state();
        let matches = state
            .active
            .as_ref()
            .is_some_and(|session| session.connection.as_ref() == connection);
        if matches {
            let session = state.active.take().expect("matching service session");
            session.cancellation.cancel();
        }
        matches
    }
}

struct SessionLease {
    gate: Arc<LiveServiceGate>,
    connection: Arc<ServiceConnectionAuthority>,
    cancellation: Cancellation,
}
impl SessionLease {
    fn revoke(&self) {
        self.gate.revoke_exact(&self.connection);
    }

    async fn revoke_after_disconnect(&self) -> io::Result<()> {
        let gate = self.gate.clone();
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || gate.revoke_exact(&connection))
            .await
            .map_err(io::Error::other)?;
        Ok(())
    }
}
impl Drop for SessionLease {
    fn drop(&mut self) {
        self.revoke();
    }
}

fn response(
    request: &ServiceWireRequest,
    instance: &str,
    boot: &str,
    result: Result<ServiceResult, ApiError>,
) -> ServiceWireResponse {
    ServiceWireResponse {
        version: PROTOCOL_VERSION,
        request_id: request.request_id.clone(),
        instance: instance.into(),
        daemon_boot: boot.into(),
        result,
    }
}

async fn send(
    stream: &mut UnixStream,
    reply: &ServiceWireResponse,
    expires: tokio::time::Instant,
    shutdown: &Cancellation,
    session_cancellation: Option<&Cancellation>,
) -> io::Result<()> {
    let encoded = encode_json(reply)?;
    tokio::select! {
        biased;
        _ = cancelled(shutdown) => Err(io::Error::new(io::ErrorKind::Interrupted, "service response interrupted by shutdown")),
        _ = async { if let Some(cancellation) = session_cancellation { cancelled(cancellation).await } else { std::future::pending().await } } => Err(io::Error::new(io::ErrorKind::Interrupted, "service response interrupted by disconnect")),
        result = tokio::time::timeout_at(expires, write_frame(stream, &encoded)) =>
            result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "service response expired"))?,
    }
}

/// Idle is unbounded. Once one byte arrives, the rest of the frame gets its
/// own five-second deadline, including the partial length prefix.
async fn next_frame(
    stream: &mut UnixStream,
    shutdown: &Cancellation,
    session_cancellation: &Cancellation,
) -> io::Result<Option<(Vec<u8>, tokio::time::Instant)>> {
    let first = tokio::select! {
        first = stream.read_u8() => match first {
            Ok(first) => first,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        },
        _ = cancelled(shutdown) => return Ok(None),
        _ = cancelled(session_cancellation) => return Ok(None),
    };
    let expires = tokio::time::Instant::now() + ORDINARY_TIMEOUT;
    let frame = tokio::select! {
        result = tokio::time::timeout_at(expires, async {
            let mut rest = [0; 3];
            stream.read_exact(&mut rest).await?;
            let length = u32::from_be_bytes([first, rest[0], rest[1], rest[2]]) as usize;
            if length == 0 || length > MAX_FRAME_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid frame length",
                ));
            }
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await?;
            Ok(body)
        }) => result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "service frame expired"))??,
        _ = cancelled(shutdown) => return Ok(None),
        _ = cancelled(session_cancellation) => return Ok(None),
    };
    Ok(Some((frame, expires)))
}

// Allowed: per-connection state handed over from the accept loop.
#[allow(clippy::too_many_arguments)]
pub async fn serve_registered(
    mut stream: UnixStream,
    first: ServiceWireRequest,
    expires: tokio::time::Instant,
    instance: String,
    boot: String,
    handler: Arc<dyn LocalService>,
    clock: Arc<dyn Clock>,
    gate: Arc<LiveServiceGate>,
    shutdown: Cancellation,
) -> io::Result<()> {
    if let Err(detail) = first.validate() {
        let code = if first.version != PROTOCOL_VERSION {
            ErrorCode::UnknownWireVersion
        } else {
            ErrorCode::Unsupported
        };
        return send(
            &mut stream,
            &response(&first, &instance, &boot, Err(service_error(code, detail))),
            expires,
            &shutdown,
            None,
        )
        .await;
    }
    if first.expected_instance != instance {
        return send(
            &mut stream,
            &response(
                &first,
                &instance,
                &boot,
                Err(service_error(
                    ErrorCode::InstanceMismatch,
                    "request instance differs from owner",
                )),
            ),
            expires,
            &shutdown,
            None,
        )
        .await;
    }
    let ServiceRequest::Register(ServiceRegister { .. }) = &first.service else {
        return send(
            &mut stream,
            &response(
                &first,
                &instance,
                &boot,
                Err(service_error(
                    ErrorCode::ServiceNotRegistered,
                    "register before service operations",
                )),
            ),
            expires,
            &shutdown,
            None,
        )
        .await;
    };
    // This reserved identity is stable across boots. C supplies its durable row.
    let author = crate::store::service_substrate::reserved_author_id(&instance);
    let (connection, session_cancellation) =
        match gate.register_session(&instance, &boot, author.clone(), clock.utc_now()) {
            Ok(connection) => connection,
            Err(error) => {
                return send(
                    &mut stream,
                    &response(&first, &instance, &boot, Err(error)),
                    expires,
                    &shutdown,
                    None,
                )
                .await;
            }
        };
    let lease = SessionLease {
        gate,
        connection,
        cancellation: session_cancellation,
    };
    send(
        &mut stream,
        &response(
            &first,
            &instance,
            &boot,
            Ok(ServiceResult::Registered(ServiceRegistration {
                author,
                daemon_boot: boot.clone(),
                connection_generation: lease.connection.generation(),
            })),
        ),
        expires,
        &shutdown,
        Some(&lease.cancellation),
    )
    .await?;
    loop {
        let Some((frame, expires)) =
            next_frame(&mut stream, &shutdown, &lease.cancellation).await?
        else {
            return Ok(());
        };
        let request = ServiceWireRequest::decode(&frame)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if request.expected_instance != instance {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "service instance changed",
            ));
        }
        let ServiceRequest::Operation(operation) = request.service.clone() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "duplicate service registration",
            ));
        };
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
        let handler = handler.clone();
        let gate = lease.gate.clone();
        let connection = lease.connection.clone();
        let mut work = tokio::task::spawn_blocking(move || {
            handler.service_operation(operation, &connection, gate.as_ref(), &budget)
        });
        let result = tokio::select! {
            joined = &mut work => Some(joined.map_err(io::Error::other)?),
            _ = tokio::time::sleep_until(expires) => None,
            _ = stream.read_u8() => None,
            _ = cancelled(&shutdown) => None,
            _ = cancelled(&lease.cancellation) => None,
        };
        let Some(result) = result else {
            cancellation.cancel();
            lease.revoke_after_disconnect().await?;
            drop(stream);
            let _ = work.await;
            return Ok(());
        };
        send(
            &mut stream,
            &response(&request, &instance, &boot, result),
            expires,
            &shutdown,
            Some(&lease.cancellation),
        )
        .await?;
    }
}

#[cfg(test)]
mod send_tests {
    use super::*;

    fn reply(result: Result<ServiceResult, ApiError>) -> ServiceWireResponse {
        let daemon_boot = match &result {
            Ok(ServiceResult::Registered(registration)) => registration.daemon_boot.clone(),
            _ => Uuid::new_v4().to_string(),
        };
        ServiceWireResponse {
            version: PROTOCOL_VERSION,
            request_id: "registered-send".into(),
            instance: Uuid::new_v4().to_string(),
            daemon_boot,
            result,
        }
    }

    #[tokio::test]
    async fn shutdown_preempts_small_rejection_and_registration_responses() {
        let error = reply(Err(service_error(ErrorCode::ServiceBusy, "busy")));
        let registered = reply(Ok(ServiceResult::Registered(ServiceRegistration {
            author: ServiceAuthorId::new("graph"),
            daemon_boot: Uuid::new_v4().to_string(),
            connection_generation: 1,
        })));
        for response in [&error, &registered] {
            let (mut writer, reader) = UnixStream::pair().unwrap();
            let shutdown = Cancellation::default();
            shutdown.cancel();
            let result = send(
                &mut writer,
                response,
                tokio::time::Instant::now() + Duration::from_secs(1),
                &shutdown,
                None,
            )
            .await;
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
            let mut byte = [0];
            assert_eq!(
                reader.try_read(&mut byte).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }
    }

    #[tokio::test]
    async fn blocked_send_keeps_one_absolute_deadline() {
        let (mut writer, _unread_peer) = UnixStream::pair().unwrap();
        let large = reply(Err(ApiError {
            code: ErrorCode::Unsupported,
            detail: "x".repeat(MAX_FRAME_BYTES - 4_096),
            restart_argv: None,
            required_minimum_bytes: None,
        }));
        let shutdown = Cancellation::default();
        let expires = tokio::time::Instant::now() + Duration::from_millis(100);
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            send(&mut writer, &large, expires, &shutdown, None),
        )
        .await
        .expect("send must obey the request deadline");
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }
}
