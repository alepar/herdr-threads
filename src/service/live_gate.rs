//! Live authority for the one registered programmatic connection.

use crate::{
    ports::{
        ServiceAuthorityGate, ServiceConnectionAuthority, ServiceDecisionGuard,
        ServiceWriteTransactionProof,
    },
    protocol::{
        ids::ServiceAuthorId,
        results::{ApiError, ErrorCode, ServiceConnectionInspection},
        time::{Cancellation, UtcMillis},
    },
};
use std::sync::{Arc, Mutex, MutexGuard};

struct RegisteredSession {
    connection: Arc<ServiceConnectionAuthority>,
    registered_at: UtcMillis,
    cancellation: Cancellation,
}

pub(crate) struct LiveState {
    next_generation: u64,
    active: Option<RegisteredSession>,
    last_registered_at: Option<UtcMillis>,
}

/// The mutex orders revocation against a store decision guard. No database or
/// external call is made while registering or revoking.
pub struct LiveServiceGate(Mutex<LiveState>);

impl Default for LiveServiceGate {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveServiceGate {
    pub fn new() -> Self {
        Self(Mutex::new(LiveState {
            next_generation: 0,
            active: None,
            last_registered_at: None,
        }))
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, LiveState> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn register_session(
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
    ApiError::new(code, detail.to_owned())
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
        let state = self.state();
        Self::revoke_locked(state, connection)
    }
}
impl LiveServiceGate {
    /// Non-blocking `revoke_exact`: `Err(WouldBlock)` while a decision guard
    /// (or any other holder) has the gate, so the caller can defer the revoke.
    pub(crate) fn try_revoke_exact(
        &self,
        connection: &ServiceConnectionAuthority,
    ) -> Result<bool, std::sync::TryLockError<()>> {
        match self.0.try_lock() {
            Ok(state) => Ok(Self::revoke_locked(state, connection)),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                Ok(Self::revoke_locked(poisoned.into_inner(), connection))
            }
            Err(std::sync::TryLockError::WouldBlock) => Err(std::sync::TryLockError::WouldBlock),
        }
    }

    fn revoke_locked(
        mut state: MutexGuard<'_, LiveState>,
        connection: &ServiceConnectionAuthority,
    ) -> bool {
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
