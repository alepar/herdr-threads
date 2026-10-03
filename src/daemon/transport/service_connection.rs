//! Live authority for the one registered programmatic connection.

use super::*;
use crate::{
    ports::{ServiceAuthorityGate, ServiceConnectionAuthority},
    protocol::service::{
        ServiceRegister, ServiceRegistration, ServiceRequest, ServiceResult, ServiceWireRequest,
        ServiceWireResponse,
    },
    service::live_gate::LiveServiceGate,
};

fn service_error(code: ErrorCode, detail: &str) -> ApiError {
    api_error(code, detail)
}

struct SessionLease {
    gate: Arc<LiveServiceGate>,
    connection: Arc<ServiceConnectionAuthority>,
    cancellation: Cancellation,
}
impl SessionLease {
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
    /// Never blocks the dropping thread on the gate: a decision guard can hold
    /// it across a store transaction. On contention the session is cancelled at
    /// once and the revoke (which still waits for the in-flight decision) runs
    /// on a blocking thread. That thread carries no lane origin, so its commit
    /// counts as request origin (ht-p03.39).
    fn drop(&mut self) {
        if self.gate.try_revoke_exact(&self.connection).is_ok() {
            return;
        }
        self.cancellation.cancel();
        let gate = self.gate.clone();
        let connection = self.connection.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn_blocking(move || gate.revoke_exact(&connection));
            }
            Err(_) => {
                std::thread::spawn(move || gate.revoke_exact(&connection));
            }
        }
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
        _ = shutdown.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "service response interrupted by shutdown")),
        _ = async { if let Some(cancellation) = session_cancellation { cancellation.cancelled().await } else { std::future::pending().await } } => Err(io::Error::new(io::ErrorKind::Interrupted, "service response interrupted by disconnect")),
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
        _ = shutdown.cancelled() => return Ok(None),
        _ = session_cancellation.cancelled() => return Ok(None),
    };
    let expires =
        tokio::time::Instant::now() + crate::protocol::time::external_bound(ORDINARY_TIMEOUT);
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
        _ = shutdown.cancelled() => return Ok(None),
        _ = session_cancellation.cancelled() => return Ok(None),
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
            _ = shutdown.cancelled() => None,
            _ = lease.cancellation.cancelled() => None,
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
#[path = "../../../tests/daemon/session_lease.rs"]
mod session_lease_tests;

#[cfg(test)]
mod send_tests {
    use super::*;
    use crate::protocol::ids::ServiceAuthorId;

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
        let large = reply(Err(ApiError::unsupported(
            "x".repeat(MAX_FRAME_BYTES - 4_096),
        )));
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
