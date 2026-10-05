//! Persistent, connection-scoped service client. Mutations are journaled before
//! the first byte is submitted and are retried only by an explicit caller action.

use crate::{
    daemon::transport::{MAX_FRAME_BYTES, encode_json, read_frame},
    protocol::{
        ids::{OperationId, ServiceAuthorId},
        pagination::Page,
        results::{ApiError, DeliveryInspection, ErrorCode, MessageSummary},
        service::{
            SERVICE_SESSION_CAPABILITY_V2, ServiceHistoryQuery, ServiceMessageSent,
            ServiceOperation, ServiceReceiptsQuery, ServiceRegister, ServiceRegistration,
            ServiceRequest, ServiceResult, ServiceSend, ServiceWireRequest, ServiceWireResponse,
        },
        time::{CallBudget, Clock},
        wire::PROTOCOL_VERSION,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    net::UnixStream,
    sync::{Mutex, MutexGuard},
};
use uuid::Uuid;

fn error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError::new(code, detail)
}

// Allowed: an error type returned once per call; ServiceResult carries the v2 receipts
// inspection and boxing it would change the wire-contract type.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum ServiceCallError {
    Api {
        error: ApiError,
        pending: Option<OperationId>,
    },
    Journal(io::Error),
    /// The daemon answered definitively, but local completion needs repair.
    Completion {
        key: OperationId,
        result: Result<ServiceResult, ApiError>,
        source: io::Error,
    },
}
impl ServiceCallError {
    /// A pending operation must be inspected or explicitly replayed after reconnect.
    pub fn pending_operation(&self) -> Option<&OperationId> {
        match self {
            Self::Api { pending, .. } => pending.as_ref(),
            Self::Journal(_) | Self::Completion { .. } => None,
        }
    }
    pub fn definitive_completion(
        &self,
    ) -> Option<(&OperationId, &Result<ServiceResult, ApiError>)> {
        match self {
            Self::Completion { key, result, .. } => Some((key, result)),
            _ => None,
        }
    }
}
impl From<io::Error> for ServiceCallError {
    fn from(error: io::Error) -> Self {
        Self::Journal(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedServiceIntent {
    pub author: ServiceAuthorId,
    pub request: ServiceWireRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedServiceIntent {
    pub key: OperationId,
    pub saved: SavedServiceIntent,
    pub response: ServiceWireResponse,
}

/// Private file-per-operation journal. A successful `record` has synced the
/// file and directory before the caller can submit the request.
pub struct ServiceIntentJournal {
    root: PathBuf,
    #[cfg(test)]
    fault: std::sync::Mutex<Option<&'static str>>,
}
impl ServiceIntentJournal {
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref();
        if !root.exists() {
            #[cfg(unix)]
            {
                fs::DirBuilder::new().mode(0o700).create(root)?;
            }
            #[cfg(not(unix))]
            {
                fs::create_dir(root)?;
            }
            let parent = root
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            File::open(parent)?.sync_all()?;
        }
        if !root.is_dir() || root.symlink_metadata()?.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe service intent directory",
            ));
        }
        #[cfg(unix)]
        if root.metadata()?.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "service intent directory is not private",
            ));
        }
        Ok(Self {
            root: root.canonicalize()?,
            #[cfg(test)]
            fault: std::sync::Mutex::new(None),
        })
    }
    #[cfg(test)]
    fn fail_at(&self, stage: &'static str) {
        *self.fault.lock().unwrap() = Some(stage);
    }
    fn boundary(&self, stage: &'static str) -> io::Result<()> {
        #[cfg(test)]
        if self
            .fault
            .lock()
            .unwrap()
            .take_if(|next| *next == stage)
            .is_some()
        {
            return Err(io::Error::other(format!("injected {stage}")));
        }
        let _ = stage;
        Ok(())
    }
    fn path(&self, key: &OperationId) -> io::Result<PathBuf> {
        Ok(self.root.join(format!(
            "{:x}.intent",
            Sha256::digest(key.as_str().as_bytes())
        )))
    }
    fn completion_path(&self, key: &OperationId) -> PathBuf {
        self.root.join(format!(
            "{:x}.complete",
            Sha256::digest(key.as_str().as_bytes())
        ))
    }
    fn record(&self, key: &OperationId, saved: &SavedServiceIntent) -> io::Result<()> {
        if self.completion_path(key).exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "operation key already completed",
            ));
        }
        let path = self.path(key)?;
        let temporary = self.root.join(format!(".{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&temporary)?;
            serde_json::to_writer(&mut file, saved)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            fs::hard_link(&temporary, &path)?; // create_new semantics for the operation key
            File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        let _ = fs::remove_file(&temporary);
        result
    }
    pub fn inspect(&self, key: &OperationId) -> io::Result<SavedServiceIntent> {
        let bytes = fs::read(self.path(key)?)?;
        let saved: SavedServiceIntent = serde_json::from_slice(&bytes)?;
        let ServiceRequest::Operation(operation) = &saved.request.service else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "journal contains registration",
            ));
        };
        if saved.request.validate().is_err() || operation.operation_key() != Some(key) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid journaled service request",
            ));
        }
        Ok(saved)
    }
    pub fn inspect_completed(&self, key: &OperationId) -> io::Result<CompletedServiceIntent> {
        let bytes = fs::read(self.completion_path(key))?;
        let completed: CompletedServiceIntent = serde_json::from_slice(&bytes)?;
        if &completed.key != key || completed.saved.request.validate().is_err() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid completed service record",
            ));
        }
        let ServiceRequest::Operation(operation) = &completed.saved.request.service else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "completion contains registration",
            ));
        };
        if operation.operation_key() != Some(key)
            || !completed.response.correlates_to(&completed.saved.request)
            || completed
                .response
                .result
                .as_ref()
                .is_ok_and(|result| !result_matches(&completed.saved.request, result))
            || completed
                .response
                .result
                .as_ref()
                .is_err_and(|failure| failure.code == ErrorCode::UnknownOutcome)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "completion identity or result mismatch",
            ));
        }
        match self.inspect(key) {
            Ok(saved) if saved != completed.saved => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "conflicting intent and completion",
                ));
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        Ok(completed)
    }
    fn publish_completion(&self, completed: &CompletedServiceIntent) -> io::Result<()> {
        let path = self.completion_path(&completed.key);
        if path.exists() {
            if self.inspect_completed(&completed.key)? == *completed {
                return Ok(());
            }
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "conflicting completion",
            ));
        }
        let temporary = self.root.join(format!(".{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&temporary)?;
            serde_json::to_writer(&mut file, completed)?;
            file.write_all(b"\n")?;
            self.boundary("before_completion_file_sync")?;
            file.sync_all()?;
            self.boundary("after_completion_file_sync")?;
            fs::hard_link(&temporary, &path)?;
            self.boundary("before_completion_dir_sync")?;
            File::open(&self.root)?.sync_all()?;
            self.boundary("after_completion_dir_sync")?;
            Ok(())
        })();
        let _ = fs::remove_file(&temporary);
        result
    }
    fn cleanup_completed(&self, key: &OperationId) -> io::Result<()> {
        self.inspect_completed(key)?;
        // A prior publication may have linked the completion but failed its
        // directory sync. Establish that link durably before removing intent.
        self.boundary("before_recovery_completion_dir_sync")?;
        File::open(&self.root)?.sync_all()?;
        match fs::remove_file(self.path(key)?) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
        self.boundary("after_unlink")?;
        File::open(&self.root)?.sync_all()?;
        self.boundary("after_unlink_dir_sync")
    }
    fn complete(&self, key: &OperationId, response: ServiceWireResponse) -> io::Result<()> {
        let saved = self.inspect(key)?;
        let completed = CompletedServiceIntent {
            key: key.clone(),
            saved,
            response,
        };
        if !completed.response.correlates_to(&completed.saved.request)
            || completed
                .response
                .result
                .as_ref()
                .is_ok_and(|result| !result_matches(&completed.saved.request, result))
            || completed
                .response
                .result
                .as_ref()
                .is_err_and(|failure| failure.code == ErrorCode::UnknownOutcome)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "uncorrelated completion",
            ));
        }
        self.publish_completion(&completed)?;
        self.cleanup_completed(key)
    }
    /// Retry cleanup after a locally failed completion. The completion remains authoritative.
    pub fn retry_cleanup(&self, key: &OperationId) -> io::Result<()> {
        self.cleanup_completed(key)
    }
    /// Explicitly forget a definitive completion after its retained intent is cleaned up.
    pub fn forget_completed(&self, key: &OperationId) -> io::Result<()> {
        self.cleanup_completed(key)?;
        fs::remove_file(self.completion_path(key))?;
        File::open(&self.root)?.sync_all()
    }
    pub fn completed(&self) -> io::Result<Vec<OperationId>> {
        let mut keys = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_name().to_string_lossy().ends_with(".complete") {
                continue;
            }
            let bytes = fs::read(entry.path())?;
            let completed: CompletedServiceIntent = serde_json::from_slice(&bytes)?;
            if entry.path() != self.completion_path(&completed.key) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "completion filename mismatch",
                ));
            }
            self.inspect_completed(&completed.key)?;
            keys.push(completed.key);
        }
        keys.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(keys)
    }
    /// Lists unresolved keys, including requests whose response was lost.
    pub fn pending(&self) -> io::Result<Vec<OperationId>> {
        let mut keys = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".intent") {
                let bytes = fs::read(entry.path())?;
                let saved: SavedServiceIntent = serde_json::from_slice(&bytes)?;
                let ServiceRequest::Operation(operation) = &saved.request.service else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "journal contains registration",
                    ));
                };
                let key = operation
                    .operation_key()
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "journal contains query")
                    })?
                    .clone();
                if entry.path() != self.path(&key)? {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "service intent filename mismatch",
                    ));
                }
                self.inspect(&key)?;
                if self.completion_path(&key).exists() {
                    self.inspect_completed(&key)?;
                } else {
                    keys.push(key);
                }
            }
        }
        keys.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(keys)
    }
}

struct Session {
    stream: UnixStream,
    registration: ServiceRegistration,
}

/// One serialized request at a time; an idle registered socket has no call timer.
pub struct PersistentServiceClient {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    instance: Uuid,
    expected_boot: Option<Uuid>,
    journal: ServiceIntentJournal,
    session: Mutex<Option<Session>>,
}
impl PersistentServiceClient {
    pub fn new(
        path: PathBuf,
        clock: Arc<dyn Clock>,
        instance: Uuid,
        expected_boot: Option<Uuid>,
        journal: ServiceIntentJournal,
    ) -> Self {
        Self {
            path,
            clock,
            instance,
            expected_boot,
            journal,
            session: Mutex::new(None),
        }
    }
    pub fn journal(&self) -> &ServiceIntentJournal {
        &self.journal
    }
    fn remaining(&self, budget: &CallBudget) -> Result<Duration, ApiError> {
        if budget.cancellation.is_cancelled() {
            return Err(error(ErrorCode::Cancelled, "request cancelled"));
        }
        let ms = budget
            .deadline
            .0
            .saturating_sub(self.clock.monotonic_now().0);
        if ms == 0 {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "request deadline elapsed",
            ));
        }
        Ok(Duration::from_millis(ms))
    }
    fn request(&self, service: ServiceRequest) -> ServiceWireRequest {
        ServiceWireRequest {
            version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4().to_string(),
            expected_instance: self.instance.to_string(),
            service,
        }
    }
    async fn admit(
        &self,
        budget: &CallBudget,
    ) -> Result<MutexGuard<'_, Option<Session>>, ApiError> {
        let deadline = tokio::time::Instant::now() + self.remaining(budget)?;
        let held = tokio::select! {
            biased;
            _ = budget.cancellation.cancelled() => return Err(error(ErrorCode::Cancelled, "request cancelled while waiting for session")),
            _ = tokio::time::sleep_until(deadline) => return Err(error(ErrorCode::DeadlineExceeded, "request deadline elapsed while waiting for session")),
            held = self.session.lock() => held,
        };
        self.remaining(budget)?;
        Ok(held)
    }
    async fn write<W: AsyncWrite + Unpin>(
        &self,
        stream: &mut W,
        body: &[u8],
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        if body.is_empty() || body.len() > MAX_FRAME_BYTES {
            return Err(error(
                ErrorCode::InvalidRequest,
                "invalid request frame length",
            ));
        }
        let deadline = tokio::time::Instant::now() + self.remaining(budget)?;
        let prefix = (body.len() as u32).to_be_bytes();
        for mut part in [&prefix[..], body] {
            while !part.is_empty() {
                let written = tokio::select! {
                    result = tokio::time::timeout_at(deadline, stream.write(part)) => result.map_err(|_| error(ErrorCode::HostUnavailable, "incomplete request frame"))?.map_err(|_| error(ErrorCode::HostUnavailable, "incomplete request frame"))?,
                    _ = budget.cancellation.cancelled() => return Err(error(ErrorCode::HostUnavailable, "incomplete request frame")),
                };
                if written == 0 {
                    return Err(error(
                        ErrorCode::HostUnavailable,
                        "incomplete request frame",
                    ));
                }
                part = &part[written..];
            }
        }
        Ok(())
    }
    async fn exchange(
        &self,
        stream: &mut UnixStream,
        request: &ServiceWireRequest,
        budget: &CallBudget,
    ) -> Result<ServiceWireResponse, ApiError> {
        request
            .validate()
            .map_err(|detail| error(ErrorCode::InvalidRequest, detail))?;
        let body = encode_json(request)
            .map_err(|_| error(ErrorCode::InvalidRequest, "request exceeds frame limit"))?;
        self.write(stream, &body, budget).await?;
        let deadline = tokio::time::Instant::now()
            + self.remaining(budget).map_err(|_| {
                error(
                    ErrorCode::UnknownOutcome,
                    "unknown outcome after request submission",
                )
            })?;
        let bytes = tokio::select! {
            result = read_frame(stream) => result.map_err(|_| error(ErrorCode::UnknownOutcome, "unknown outcome after request submission"))?,
            _ = tokio::time::sleep_until(deadline) => return Err(error(ErrorCode::UnknownOutcome, "unknown outcome after request submission")),
            _ = budget.cancellation.cancelled() => return Err(error(ErrorCode::UnknownOutcome, "unknown outcome after request submission")),
        };
        let response: ServiceWireResponse = serde_json::from_slice(&bytes)
            .map_err(|_| error(ErrorCode::UnknownOutcome, "invalid service response"))?;
        if !response.correlates_to(request)
            || self
                .expected_boot
                .is_some_and(|boot| boot.to_string() != response.daemon_boot)
        {
            return Err(error(
                ErrorCode::UnknownOutcome,
                "service response identity mismatch",
            ));
        }
        Ok(response)
    }
    /// Registration is connection-local. An ambiguous response closes the socket;
    /// the caller can reconnect after that close, without a durable mutation key.
    /// Registers `service_session_v2`: an old daemon rejects it with
    /// "unsupported service capability" and there is no silent fallback to v1.
    pub async fn register(&self, budget: &CallBudget) -> Result<ServiceRegistration, ApiError> {
        let mut held = self.admit(budget).await?;
        if let Some(session) = held.as_ref() {
            return Ok(session.registration.clone());
        }
        let remaining = self.remaining(budget)?.min(Duration::from_secs(2));
        let mut stream = tokio::select! {
            result = tokio::time::timeout(remaining, UnixStream::connect(&self.path)) => result.map_err(|_| error(ErrorCode::HostUnavailable, "daemon connect timed out"))?.map_err(|connect| super::connect_error(&connect, &self.path))?,
            _ = budget.cancellation.cancelled() => return Err(error(ErrorCode::Cancelled, "request cancelled before connect")),
        };
        let request = self.request(ServiceRequest::Register(ServiceRegister {
            capability: SERVICE_SESSION_CAPABILITY_V2.into(),
        }));
        let response = self.exchange(&mut stream, &request, budget).await?;
        match response.result {
            Ok(ServiceResult::Registered(registration)) => {
                *held = Some(Session {
                    stream,
                    registration: registration.clone(),
                });
                Ok(registration)
            }
            Ok(_) => Err(error(
                ErrorCode::UnknownOutcome,
                "unexpected registration result",
            )),
            Err(error) => Err(error),
        }
    }
    pub async fn disconnect(&self) {
        *self.session.lock().await = None;
    }
    pub async fn registration(&self) -> Option<ServiceRegistration> {
        self.session
            .lock()
            .await
            .as_ref()
            .map(|s| s.registration.clone())
    }
    /// Queries require a live registration and are never journaled.
    pub async fn query(
        &self,
        operation: ServiceOperation,
        budget: &CallBudget,
    ) -> Result<ServiceResult, ApiError> {
        if operation.operation_key().is_some() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "mutation requires durable submission",
            ));
        }
        let mut held = self.admit(budget).await?;
        let session = held
            .as_mut()
            .ok_or_else(|| error(ErrorCode::ServiceNotRegistered, "register service first"))?;
        let request = self.request(ServiceRequest::Operation(operation));
        let expected_boot = session.registration.daemon_boot.clone();
        let response = self.exchange(&mut session.stream, &request, budget).await;
        match response {
            Ok(response) if response.daemon_boot == expected_boot => match response.result {
                Ok(result) if result_matches(&request, &result) => Ok(result),
                Ok(_) => {
                    *held = None;
                    Err(error(ErrorCode::UnknownOutcome, "unexpected query result"))
                }
                Err(error) => {
                    if invalidates_session(&error) {
                        *held = None;
                    }
                    Err(error)
                }
            },
            Ok(_) => {
                *held = None;
                Err(error(ErrorCode::UnknownOutcome, "daemon boot changed"))
            }
            Err(error) => {
                *held = None;
                Err(error)
            }
        }
    }
    /// Persist the exact envelope, key and payload before its first submission.
    pub async fn submit(
        &self,
        operation: ServiceOperation,
        budget: &CallBudget,
    ) -> Result<(OperationId, ServiceResult), ServiceCallError> {
        let key = operation
            .operation_key()
            .cloned()
            .ok_or_else(|| ServiceCallError::Api {
                error: error(ErrorCode::InvalidRequest, "query has no mutation key"),
                pending: None,
            })?;
        let mut held = self
            .admit(budget)
            .await
            .map_err(|error| ServiceCallError::Api {
                error,
                pending: None,
            })?;
        let session = held.as_mut().ok_or_else(|| ServiceCallError::Api {
            error: error(ErrorCode::ServiceNotRegistered, "register service first"),
            pending: None,
        })?;
        let request = self.request(ServiceRequest::Operation(operation));
        request.validate().map_err(|detail| ServiceCallError::Api {
            error: error(ErrorCode::InvalidRequest, detail),
            pending: None,
        })?;
        encode_json(&request).map_err(|_| ServiceCallError::Api {
            error: error(ErrorCode::InvalidRequest, "request exceeds frame limit"),
            pending: None,
        })?;
        self.remaining(budget)
            .map_err(|error| ServiceCallError::Api {
                error,
                pending: None,
            })?;
        self.journal.record(
            &key,
            &SavedServiceIntent {
                author: session.registration.author.clone(),
                request: request.clone(),
            },
        )?;
        self.remaining(budget)
            .map_err(|error| ServiceCallError::Api {
                error,
                pending: Some(key.clone()),
            })?;
        self.send_saved(&mut held, &key, &request, budget).await
    }
    /// Journaled send into a managed thread; the key is the request's operation.
    pub async fn send(
        &self,
        request: ServiceSend,
        budget: &CallBudget,
    ) -> Result<(OperationId, ServiceMessageSent), ServiceCallError> {
        let (key, result) = self.submit(ServiceOperation::Send(request), budget).await?;
        match result {
            ServiceResult::MessageSent(sent) => Ok((key, sent)),
            _ => Err(ServiceCallError::Api {
                error: error(ErrorCode::UnknownOutcome, "unexpected service result"),
                pending: Some(key),
            }),
        }
    }
    /// Read the history of any thread in the instance; never journaled.
    pub async fn history(
        &self,
        query: ServiceHistoryQuery,
        budget: &CallBudget,
    ) -> Result<Page<MessageSummary>, ApiError> {
        match self.query(ServiceOperation::History(query), budget).await? {
            ServiceResult::History(page) => Ok(page),
            _ => Err(error(
                ErrorCode::UnknownOutcome,
                "unexpected service result",
            )),
        }
    }
    /// Read the receipt state of one of this author's messages; never journaled.
    pub async fn receipts(
        &self,
        query: ServiceReceiptsQuery,
        budget: &CallBudget,
    ) -> Result<DeliveryInspection, ApiError> {
        match self
            .query(ServiceOperation::Receipts(query), budget)
            .await?
        {
            ServiceResult::Receipts(inspection) => Ok(inspection),
            _ => Err(error(
                ErrorCode::UnknownOutcome,
                "unexpected service result",
            )),
        }
    }
    /// Explicit exact-envelope replay after the caller has assessed uncertainty.
    pub async fn replay(
        &self,
        key: &OperationId,
        budget: &CallBudget,
    ) -> Result<ServiceResult, ServiceCallError> {
        let mut held = self
            .admit(budget)
            .await
            .map_err(|error| ServiceCallError::Api {
                error,
                pending: Some(key.clone()),
            })?;
        let session = held.as_ref().ok_or_else(|| ServiceCallError::Api {
            error: error(ErrorCode::ServiceNotRegistered, "register service first"),
            pending: Some(key.clone()),
        })?;
        if self.journal.completion_path(key).exists() {
            self.journal.inspect_completed(key)?;
            return Err(ServiceCallError::Api {
                error: error(ErrorCode::InvalidRequest, "operation already completed"),
                pending: None,
            });
        }
        let saved = self.journal.inspect(key)?;
        if saved.author != session.registration.author
            || saved.request.expected_instance != self.instance.to_string()
        {
            return Err(ServiceCallError::Api {
                error: error(
                    ErrorCode::Unauthorized,
                    "service intent authority scope mismatch",
                ),
                pending: Some(key.clone()),
            });
        }
        self.remaining(budget)
            .map_err(|error| ServiceCallError::Api {
                error,
                pending: Some(key.clone()),
            })?;
        self.send_saved(&mut held, key, &saved.request, budget)
            .await
            .map(|(_, result)| result)
    }
    async fn send_saved(
        &self,
        held: &mut Option<Session>,
        key: &OperationId,
        request: &ServiceWireRequest,
        budget: &CallBudget,
    ) -> Result<(OperationId, ServiceResult), ServiceCallError> {
        let session = held.as_mut().expect("registered session checked by caller");
        let boot = session.registration.daemon_boot.clone();
        let response = self.exchange(&mut session.stream, request, budget).await;
        let response = match response {
            Ok(response) if response.daemon_boot == boot => response,
            Ok(_) => {
                *held = None;
                return Err(ServiceCallError::Api {
                    error: error(ErrorCode::UnknownOutcome, "daemon boot changed"),
                    pending: Some(key.clone()),
                });
            }
            Err(failure) => {
                *held = None;
                return Err(ServiceCallError::Api {
                    error: failure,
                    pending: Some(key.clone()),
                });
            }
        };
        let definitive_response = response.clone();
        match response.result {
            Ok(result) => {
                if !result_matches(request, &result) {
                    *held = None;
                    return Err(ServiceCallError::Api {
                        error: error(ErrorCode::UnknownOutcome, "unexpected mutation result"),
                        pending: Some(key.clone()),
                    });
                }
                self.journal
                    .complete(key, definitive_response)
                    .map_err(|source| ServiceCallError::Completion {
                        key: key.clone(),
                        result: Ok(result.clone()),
                        source,
                    })?;
                Ok((key.clone(), result))
            }
            Err(failure) => {
                if failure.code == ErrorCode::UnknownOutcome {
                    *held = None;
                    return Err(ServiceCallError::Api {
                        error: failure,
                        pending: Some(key.clone()),
                    });
                }
                if invalidates_session(&failure) {
                    *held = None;
                }
                self.journal
                    .complete(key, definitive_response)
                    .map_err(|source| ServiceCallError::Completion {
                        key: key.clone(),
                        result: Err(failure.clone()),
                        source,
                    })?;
                Err(ServiceCallError::Api {
                    error: failure,
                    pending: None,
                })
            }
        }
    }
}

fn invalidates_session(error: &ApiError) -> bool {
    matches!(
        error.code,
        ErrorCode::ServiceNotRegistered
            | ErrorCode::StaleServiceGeneration
            | ErrorCode::InstanceMismatch
            | ErrorCode::DaemonVersionMismatch
            | ErrorCode::UnknownWireVersion
    )
}

fn result_matches(request: &ServiceWireRequest, result: &ServiceResult) -> bool {
    matches!(
        (&request.service, result),
        (
            ServiceRequest::Operation(ServiceOperation::EnsureThread(_)),
            ServiceResult::ThreadEnsured(_)
        ) | (
            ServiceRequest::Operation(ServiceOperation::Invite(_)),
            ServiceResult::Invitation(_) | ServiceResult::AlreadyJoined(_)
        ) | (
            ServiceRequest::Operation(ServiceOperation::Notify(_)),
            ServiceResult::Notification(_)
        ) | (
            ServiceRequest::Operation(ServiceOperation::Membership(_)),
            ServiceResult::Membership(_)
        ) | (
            ServiceRequest::Operation(ServiceOperation::SetTopic(_)),
            ServiceResult::TopicChanged(_)
        ) | (
            ServiceRequest::Operation(ServiceOperation::ReleaseRequirement(_)),
            ServiceResult::RequirementReleased(_)
        ) | (
            ServiceRequest::Operation(ServiceOperation::Archive(_)),
            ServiceResult::Archived(_)
        ) | (
            ServiceRequest::Operation(ServiceOperation::Reopen(_)),
            ServiceResult::Reopened(_)
        ) | (
            ServiceRequest::Operation(ServiceOperation::Send(_)),
            ServiceResult::MessageSent(_)
        ) | (
            ServiceRequest::Operation(ServiceOperation::History(_)),
            ServiceResult::History(_)
        ) | (
            ServiceRequest::Operation(ServiceOperation::Receipts(_)),
            ServiceResult::Receipts(_)
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        ids::ThreadId,
        pagination::PageRequest,
        service::{EnsureManagedThread, ManagedThread, ServiceMembershipQuery},
        time::{Cancellation, MonoInstant, UtcMillis},
    };
    use tokio::net::UnixListener;

    const INSTANCE: &str = "123e4567-e89b-12d3-a456-426614174000";
    const BOOT: &str = "123e4567-e89b-12d3-a456-426614174001";
    const OTHER_BOOT: &str = "123e4567-e89b-12d3-a456-426614174002";

    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(0)
        }
    }
    struct ProgressClock(std::time::Instant);
    impl Clock for ProgressClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.elapsed().as_millis() as u64)
        }
    }
    struct AdjustableClock(std::sync::atomic::AtomicU64);
    impl Clock for AdjustableClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(std::sync::atomic::Ordering::SeqCst))
        }
    }
    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(1000),
            cancellation: Cancellation::default(),
        }
    }
    fn temp() -> PathBuf {
        PathBuf::from("/tmp").join(format!("hsc-{}", Uuid::new_v4()))
    }
    fn make_client(socket: PathBuf, root: &Path) -> PersistentServiceClient {
        PersistentServiceClient::new(
            socket,
            Arc::new(FixedClock),
            Uuid::parse_str(INSTANCE).unwrap(),
            None,
            ServiceIntentJournal::open(root).unwrap(),
        )
    }
    async fn incoming(stream: &mut UnixStream) -> ServiceWireRequest {
        ServiceWireRequest::decode(&read_frame(stream).await.unwrap()).unwrap()
    }
    async fn respond(
        stream: &mut UnixStream,
        request: &ServiceWireRequest,
        boot: &str,
        result: Result<ServiceResult, ApiError>,
    ) {
        let response = ServiceWireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id.clone(),
            instance: INSTANCE.into(),
            daemon_boot: boot.into(),
            result,
        };
        let body = encode_json(&response).unwrap();
        stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        stream.write_all(&body).await.unwrap();
    }
    fn registration() -> ServiceResult {
        ServiceResult::Registered(ServiceRegistration {
            author: ServiceAuthorId::new("graph"),
            daemon_boot: BOOT.into(),
            connection_generation: 1,
        })
    }
    fn ensure(key: OperationId) -> ServiceOperation {
        ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: ThreadId::new("channel"),
            topic: "topic".into(),
            goal: "goal".into(),
            operation: key,
        })
    }
    fn ensured() -> ServiceResult {
        ServiceResult::ThreadEnsured(ManagedThread {
            thread: ThreadId::new("channel"),
            owner: ServiceAuthorId::new("graph"),
            archived: false,
        })
    }
    fn definitive_response(
        saved: &SavedServiceIntent,
        result: Result<ServiceResult, ApiError>,
    ) -> ServiceWireResponse {
        ServiceWireResponse {
            version: PROTOCOL_VERSION,
            request_id: saved.request.request_id.clone(),
            instance: saved.request.expected_instance.clone(),
            daemon_boot: BOOT.into(),
            result,
        }
    }
    fn membership_query() -> ServiceOperation {
        ServiceOperation::Membership(ServiceMembershipQuery {
            thread: ThreadId::new("channel"),
            seat: None,
            page: PageRequest {
                cursor: None,
                limit: 10,
                max_bytes: 4096,
            },
        })
    }

    #[tokio::test]
    async fn queued_register_observes_its_live_budget() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let register = incoming(&mut stream).await;
            respond(&mut stream, &register, BOOT, Ok(registration())).await;
            let query = incoming(&mut stream).await;
            entered_tx.send(()).unwrap();
            release_rx.await.unwrap();
            let result = ServiceResult::Membership(crate::protocol::pagination::Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            });
            respond(&mut stream, &query, BOOT, Ok(result)).await;
            assert!(
                tokio::time::timeout(Duration::from_millis(30), read_frame(&mut stream))
                    .await
                    .is_err(),
                "rejected queued call wrote socket bytes"
            );
        });
        let progress = Arc::new(ProgressClock(std::time::Instant::now()));
        let client = Arc::new(PersistentServiceClient::new(
            socket,
            progress.clone(),
            Uuid::parse_str(INSTANCE).unwrap(),
            None,
            ServiceIntentJournal::open(root.join("intents")).unwrap(),
        ));
        let generous = CallBudget {
            deadline: MonoInstant(1000),
            cancellation: Cancellation::default(),
        };
        client.register(&generous).await.unwrap();
        let owner = {
            let client = client.clone();
            tokio::spawn(async move { client.query(membership_query(), &generous).await })
        };
        entered_rx.await.unwrap();
        let saved_key = OperationId::new("queued-replay-key");
        let saved = SavedServiceIntent {
            author: ServiceAuthorId::new("graph"),
            request: ServiceWireRequest {
                version: PROTOCOL_VERSION,
                request_id: "queued-replay-request".into(),
                expected_instance: INSTANCE.into(),
                service: ServiceRequest::Operation(ensure(saved_key.clone())),
            },
        };
        client.journal().record(&saved_key, &saved).unwrap();
        let rejected_key = OperationId::new("queued-rejected-submit");
        let short = CallBudget {
            deadline: MonoInstant(progress.monotonic_now().0 + 20),
            cancellation: Cancellation::default(),
        };
        let queued = tokio::time::timeout(Duration::from_millis(100), async {
            tokio::join!(
                client.register(&short),
                client.query(membership_query(), &short),
                client.submit(ensure(rejected_key.clone()), &short),
                client.replay(&saved_key, &short),
            )
        })
        .await;
        assert!(
            queued.is_ok(),
            "queued call waited behind owner past deadline"
        );
        let (register, query, submit, replay) = queued.unwrap();
        assert_eq!(register.unwrap_err().code, ErrorCode::DeadlineExceeded);
        assert_eq!(query.unwrap_err().code, ErrorCode::DeadlineExceeded);
        assert!(matches!(
            submit.unwrap_err(),
            ServiceCallError::Api {
                error: ApiError {
                    code: ErrorCode::DeadlineExceeded,
                    ..
                },
                pending: None
            }
        ));
        assert!(matches!(
            replay.unwrap_err(),
            ServiceCallError::Api {
                error: ApiError {
                    code: ErrorCode::DeadlineExceeded,
                    ..
                },
                pending: Some(_)
            }
        ));
        assert_eq!(client.journal().pending().unwrap(), vec![saved_key.clone()]);
        assert_eq!(client.journal().inspect(&saved_key).unwrap(), saved);
        assert!(client.journal().inspect(&rejected_key).is_err());
        let cancelled = CallBudget {
            deadline: MonoInstant(progress.monotonic_now().0 + 1000),
            cancellation: Cancellation::default(),
        };
        let token = cancelled.cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            token.cancel();
        });
        let cancelled_result =
            tokio::time::timeout(Duration::from_millis(100), client.register(&cancelled))
                .await
                .unwrap();
        assert_eq!(cancelled_result.unwrap_err().code, ErrorCode::Cancelled);
        release_tx.send(()).unwrap();
        owner.await.unwrap().unwrap();
        assert!(client.registration().await.is_some());
        assert_eq!(
            client.register(&cancelled).await.unwrap_err().code,
            ErrorCode::Cancelled
        );
        server.await.unwrap();
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn deadline_advanced_while_acquiring_lock_is_checked_again() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let clock = Arc::new(AdjustableClock(std::sync::atomic::AtomicU64::new(0)));
        let client = Arc::new(PersistentServiceClient::new(
            root.join("unused-socket"),
            clock.clone(),
            Uuid::parse_str(INSTANCE).unwrap(),
            None,
            ServiceIntentJournal::open(root.join("intents")).unwrap(),
        ));
        let held = client.session.lock().await;
        let waiter = {
            let client = client.clone();
            tokio::spawn(async move {
                client
                    .register(&CallBudget {
                        deadline: MonoInstant(1000),
                        cancellation: Cancellation::default(),
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        clock.0.store(1000, std::sync::atomic::Ordering::SeqCst);
        drop(held);
        assert_eq!(
            waiter.await.unwrap().unwrap_err().code,
            ErrorCode::DeadlineExceeded
        );
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_operation_key_cannot_replace_saved_payload() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let journal = ServiceIntentJournal::open(root.join("intents")).unwrap();
        let key = OperationId::new("graph-channel-key");
        let original = SavedServiceIntent {
            author: ServiceAuthorId::new("graph"),
            request: ServiceWireRequest {
                version: PROTOCOL_VERSION,
                request_id: "request-1".into(),
                expected_instance: INSTANCE.into(),
                service: ServiceRequest::Operation(ensure(key.clone())),
            },
        };
        journal.record(&key, &original).unwrap();
        let mut changed = original.clone();
        if let ServiceRequest::Operation(ServiceOperation::EnsureThread(request)) =
            &mut changed.request.service
        {
            request.topic = "changed".into();
        }
        assert_eq!(
            journal.record(&key, &changed).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(journal.inspect(&key).unwrap().request, original.request);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_post_unlink_sync_keeps_recoverable_intent() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let journal = ServiceIntentJournal::open(root.join("intents")).unwrap();
        let key = OperationId::new("sync-failure-key");
        let saved = SavedServiceIntent {
            author: ServiceAuthorId::new("graph"),
            request: ServiceWireRequest {
                version: PROTOCOL_VERSION,
                request_id: "request-sync-failure".into(),
                expected_instance: INSTANCE.into(),
                service: ServiceRequest::Operation(ensure(key.clone())),
            },
        };
        journal.record(&key, &saved).unwrap();
        journal.fail_at("after_unlink");
        assert!(
            journal
                .complete(&key, definitive_response(&saved, Ok(ensured())))
                .is_err()
        );
        assert!(journal.pending().unwrap().is_empty());
        assert_eq!(
            journal.inspect_completed(&key).unwrap().saved.request,
            saved.request
        );
        let reopened = ServiceIntentJournal::open(root.join("intents")).unwrap();
        assert!(reopened.pending().unwrap().is_empty());
        assert_eq!(reopened.completed().unwrap(), vec![key.clone()]);
        reopened.retry_cleanup(&key).unwrap();
        reopened.forget_completed(&key).unwrap();
        assert!(reopened.completed().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completion_fault_prefixes_reopen_as_pending_or_definitive() {
        for definitive in [Ok(ensured()), Err(error(ErrorCode::Unauthorized, "denied"))] {
            for stage in [
                "before_completion_file_sync",
                "after_completion_file_sync",
                "before_completion_dir_sync",
                "after_completion_dir_sync",
                "after_unlink",
                "after_unlink_dir_sync",
            ] {
                let root = temp();
                fs::create_dir_all(&root).unwrap();
                let journal = ServiceIntentJournal::open(root.join("intents")).unwrap();
                let key = OperationId::new("fault-prefix-key");
                let saved = SavedServiceIntent {
                    author: ServiceAuthorId::new("graph"),
                    request: ServiceWireRequest {
                        version: PROTOCOL_VERSION,
                        request_id: "request-fault-prefix".into(),
                        expected_instance: INSTANCE.into(),
                        service: ServiceRequest::Operation(ensure(key.clone())),
                    },
                };
                journal.record(&key, &saved).unwrap();
                journal.fail_at(stage);
                assert!(
                    journal
                        .complete(&key, definitive_response(&saved, definitive.clone()))
                        .is_err(),
                    "{stage}"
                );
                drop(journal);
                let reopened = ServiceIntentJournal::open(root.join("intents")).unwrap();
                let linked = !matches!(
                    stage,
                    "before_completion_file_sync" | "after_completion_file_sync"
                );
                if linked {
                    assert!(reopened.pending().unwrap().is_empty(), "{stage}");
                    assert_eq!(reopened.completed().unwrap(), vec![key.clone()], "{stage}");
                    assert_eq!(
                        reopened.inspect_completed(&key).unwrap().response.result,
                        definitive
                    );
                    if stage == "before_completion_dir_sync" {
                        reopened.fail_at("before_recovery_completion_dir_sync");
                        assert!(reopened.retry_cleanup(&key).is_err());
                        assert_eq!(reopened.inspect(&key).unwrap(), saved);
                    }
                    reopened.retry_cleanup(&key).unwrap();
                    reopened.forget_completed(&key).unwrap();
                } else {
                    assert_eq!(reopened.pending().unwrap(), vec![key.clone()], "{stage}");
                    assert!(reopened.completed().unwrap().is_empty(), "{stage}");
                }
                fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn malformed_or_conflicting_completion_is_rejected() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let journal = ServiceIntentJournal::open(root.join("intents")).unwrap();
        let key = OperationId::new("conflict-key");
        let saved = SavedServiceIntent {
            author: ServiceAuthorId::new("graph"),
            request: ServiceWireRequest {
                version: PROTOCOL_VERSION,
                request_id: "request-conflict".into(),
                expected_instance: INSTANCE.into(),
                service: ServiceRequest::Operation(ensure(key.clone())),
            },
        };
        journal.record(&key, &saved).unwrap();
        fs::write(journal.completion_path(&key), b"not-json").unwrap();
        assert_eq!(
            journal.completed().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_file(journal.completion_path(&key)).unwrap();
        let mut changed = saved.clone();
        changed.author = ServiceAuthorId::new("different");
        let response = definitive_response(&changed, Ok(ensured()));
        let completion = CompletedServiceIntent {
            key: key.clone(),
            saved: changed,
            response,
        };
        fs::write(
            journal.completion_path(&key),
            serde_json::to_vec(&completion).unwrap(),
        )
        .unwrap();
        assert_eq!(
            journal.pending().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            journal.completed().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_file(journal.completion_path(&key)).unwrap();
        let mut response = definitive_response(&saved, Ok(ensured()));
        response.request_id = "wrong-request".into();
        let completion = CompletedServiceIntent {
            key: key.clone(),
            saved,
            response,
        };
        fs::write(
            journal.completion_path(&key),
            serde_json::to_vec(&completion).unwrap(),
        )
        .unwrap();
        assert_eq!(
            journal.completed().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn post_response_completion_failure_exposes_typed_result_and_blocks_replay() {
        for result in [Ok(ensured()), Err(error(ErrorCode::Unauthorized, "denied"))] {
            let root = temp();
            fs::create_dir_all(&root).unwrap();
            let socket = root.join("socket");
            let listener = UnixListener::bind(&socket).unwrap();
            let expected = result.clone();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let register = incoming(&mut stream).await;
                respond(&mut stream, &register, BOOT, Ok(registration())).await;
                let mutation = incoming(&mut stream).await;
                respond(&mut stream, &mutation, BOOT, expected).await;
                assert!(
                    tokio::time::timeout(Duration::from_millis(30), read_frame(&mut stream))
                        .await
                        .is_err()
                );
            });
            let client = make_client(socket, &root.join("intents"));
            client.register(&budget()).await.unwrap();
            let key = OperationId::new(Uuid::new_v4().to_string());
            client.journal().fail_at("after_unlink");
            let failure = client
                .submit(ensure(key.clone()), &budget())
                .await
                .unwrap_err();
            let (reported_key, reported_result) = failure.definitive_completion().unwrap();
            assert_eq!(reported_key, &key);
            assert_eq!(reported_result, &result);
            assert!(client.journal().pending().unwrap().is_empty());
            assert_eq!(
                client
                    .journal()
                    .inspect_completed(&key)
                    .unwrap()
                    .response
                    .result,
                result
            );
            assert!(matches!(
                client.replay(&key, &budget()).await.unwrap_err(),
                ServiceCallError::Api {
                    error: ApiError {
                        code: ErrorCode::InvalidRequest,
                        ..
                    },
                    pending: None
                }
            ));
            server.await.unwrap();
            drop(client);
            let reopened = ServiceIntentJournal::open(root.join("intents")).unwrap();
            assert_eq!(reopened.completed().unwrap(), vec![key.clone()]);
            reopened.retry_cleanup(&key).unwrap();
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn idle_connection_remains_registered_and_serializes_queries() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let register = incoming(&mut stream).await;
            respond(&mut stream, &register, BOOT, Ok(registration())).await;
            tokio::time::sleep(Duration::from_millis(40)).await;
            let query = incoming(&mut stream).await;
            assert!(matches!(
                query.service,
                ServiceRequest::Operation(ServiceOperation::Membership(_))
            ));
            let result = ServiceResult::Membership(crate::protocol::pagination::Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            });
            respond(&mut stream, &query, BOOT, Ok(result)).await;
        });
        let client = make_client(socket, &root.join("intents"));
        client.register(&budget()).await.unwrap();
        let query = ServiceOperation::Membership(ServiceMembershipQuery {
            thread: ThreadId::new("channel"),
            seat: None,
            page: PageRequest {
                cursor: None,
                limit: 10,
                max_bytes: 4096,
            },
        });
        assert!(matches!(
            client.query(query, &budget()).await.unwrap(),
            ServiceResult::Membership(_)
        ));
        assert!(client.registration().await.is_some());
        server.await.unwrap();
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn lost_response_keeps_exact_envelope_until_explicit_replay() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let register = incoming(&mut first).await;
            respond(&mut first, &register, BOOT, Ok(registration())).await;
            let original = incoming(&mut first).await;
            drop(first); // commit happened, response disappeared
            let (mut second, _) = listener.accept().await.unwrap();
            let register = incoming(&mut second).await;
            respond(&mut second, &register, BOOT, Ok(registration())).await;
            let replay = incoming(&mut second).await;
            assert_eq!(original, replay);
            respond(&mut second, &replay, BOOT, Ok(ensured())).await;
        });
        let client = make_client(socket, &root.join("intents"));
        client.register(&budget()).await.unwrap();
        let key = OperationId::new("graph-ensure-channel-v1");
        let failure = client
            .submit(ensure(key.clone()), &budget())
            .await
            .unwrap_err();
        assert_eq!(failure.pending_operation(), Some(&key));
        assert!(client.registration().await.is_none());
        assert_eq!(client.journal().pending().unwrap(), vec![key.clone()]);
        let saved = client.journal().inspect(&key).unwrap();
        assert_eq!(saved.author, ServiceAuthorId::new("graph"));
        assert_eq!(
            saved.request.service,
            ServiceRequest::Operation(ensure(key.clone()))
        );
        drop(client);
        let client = make_client(root.join("socket"), &root.join("intents"));
        assert_eq!(client.journal().pending().unwrap(), vec![key.clone()]);
        client.register(&budget()).await.unwrap();
        assert_eq!(client.journal().pending().unwrap(), vec![key.clone()]); // no automatic replay
        assert_eq!(client.replay(&key, &budget()).await.unwrap(), ensured());
        assert!(client.journal().pending().unwrap().is_empty());
        server.await.unwrap();
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn changed_boot_invalidates_session_and_retains_intent() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let register = incoming(&mut stream).await;
            respond(&mut stream, &register, BOOT, Ok(registration())).await;
            let mutation = incoming(&mut stream).await;
            respond(&mut stream, &mutation, OTHER_BOOT, Ok(ensured())).await;
        });
        let client = make_client(socket, &root.join("intents"));
        client.register(&budget()).await.unwrap();
        let key = OperationId::new(Uuid::new_v4().to_string());
        let failure = client
            .submit(ensure(key.clone()), &budget())
            .await
            .unwrap_err();
        assert_eq!(failure.pending_operation(), Some(&key));
        assert!(client.registration().await.is_none());
        assert_eq!(client.journal().pending().unwrap(), vec![key]);
        server.await.unwrap();
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn wrong_correlation_and_oversized_frame_keep_mutations_pending() {
        for oversized in [false, true] {
            let root = temp();
            fs::create_dir_all(&root).unwrap();
            let socket = root.join("socket");
            let listener = UnixListener::bind(&socket).unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let register = incoming(&mut stream).await;
                respond(&mut stream, &register, BOOT, Ok(registration())).await;
                let request = incoming(&mut stream).await;
                if oversized {
                    stream
                        .write_all(&((MAX_FRAME_BYTES as u32) + 1).to_be_bytes())
                        .await
                        .unwrap();
                } else {
                    let response = ServiceWireResponse {
                        version: PROTOCOL_VERSION,
                        request_id: "different-request".into(),
                        instance: INSTANCE.into(),
                        daemon_boot: BOOT.into(),
                        result: Ok(ensured()),
                    };
                    let body = encode_json(&response).unwrap();
                    stream
                        .write_all(&(body.len() as u32).to_be_bytes())
                        .await
                        .unwrap();
                    stream.write_all(&body).await.unwrap();
                }
                let _ = request;
            });
            let client = make_client(socket, &root.join("intents"));
            client.register(&budget()).await.unwrap();
            let key = OperationId::new(Uuid::new_v4().to_string());
            let failure = client
                .submit(ensure(key.clone()), &budget())
                .await
                .unwrap_err();
            assert_eq!(failure.pending_operation(), Some(&key));
            assert!(client.registration().await.is_none());
            assert_eq!(client.journal().pending().unwrap(), vec![key]);
            server.await.unwrap();
            drop(client);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn server_side_registration_loss_clears_session() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let register = incoming(&mut stream).await;
            respond(&mut stream, &register, BOOT, Ok(registration())).await;
            let query = incoming(&mut stream).await;
            respond(
                &mut stream,
                &query,
                BOOT,
                Err(error(
                    ErrorCode::ServiceNotRegistered,
                    "registration revoked",
                )),
            )
            .await;
        });
        let client = make_client(socket, &root.join("intents"));
        client.register(&budget()).await.unwrap();
        let query = ServiceOperation::Membership(ServiceMembershipQuery {
            thread: ThreadId::new("channel"),
            seat: None,
            page: PageRequest {
                cursor: None,
                limit: 10,
                max_bytes: 4096,
            },
        });
        assert_eq!(
            client.query(query, &budget()).await.unwrap_err().code,
            ErrorCode::ServiceNotRegistered
        );
        assert!(client.registration().await.is_none());
        server.await.unwrap();
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }

    fn send_request(key: &str) -> ServiceSend {
        ServiceSend {
            thread: ThreadId::new("channel"),
            body: "do it".into(),
            recipients: vec![crate::protocol::ids::SeatId::new("seat-1")],
            deadline_millis: None,
            operation: OperationId::new(key),
        }
    }
    fn empty_page<T>() -> Page<T> {
        Page {
            items: vec![],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: crate::protocol::pagination::StopReason::Complete,
            consistency: crate::protocol::pagination::Consistency::BoundedLive,
        }
    }
    fn sent_summary() -> MessageSummary {
        MessageSummary {
            message: crate::protocol::ids::MessageId::new("m-1"),
            thread: ThreadId::new("channel"),
            author: None,
            event_author: Some(crate::protocol::service::EventAuthor::Programmatic(
                ServiceAuthorId::new("graph"),
            )),
            author_role: Some(crate::protocol::summary::AuthorRole::Service),
            relays_user: false,
            user_intent: None,
            author_role_backfilled: false,
            kind: crate::protocol::results::MessageKind::Ordinary,
            sequence: 4,
            created_at: UtcMillis(1),
            actor_label: Some("herdr-graph".into()),
            preview_data: "do it".into(),
            preview_omitted: false,
            preview_detail_argv: None,
        }
    }
    fn history_query() -> ServiceHistoryQuery {
        ServiceHistoryQuery {
            thread: ThreadId::new("channel"),
            page: PageRequest::default(),
            initial: None,
        }
    }
    fn receipts_query() -> ServiceReceiptsQuery {
        ServiceReceiptsQuery {
            message: crate::protocol::ids::MessageId::new("m-1"),
            page: PageRequest::default(),
        }
    }

    #[tokio::test]
    async fn query_refuses_send_and_submit_refuses_history() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let client = make_client(root.join("missing-socket"), &root.join("intents"));
        let error = client
            .query(ServiceOperation::Send(send_request("send-1")), &budget())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        for operation in [
            ServiceOperation::History(history_query()),
            ServiceOperation::Receipts(receipts_query()),
        ] {
            match client.submit(operation, &budget()).await {
                Err(ServiceCallError::Api { error, pending }) => {
                    assert_eq!(error.code, ErrorCode::InvalidRequest);
                    assert_eq!(pending, None);
                }
                other => panic!("expected an invalid-request refusal, got {other:?}"),
            }
        }
        assert!(client.journal().pending().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn register_negotiates_service_session_v2() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let register = incoming(&mut stream).await;
            assert!(
                matches!(&register.service, ServiceRequest::Register(ServiceRegister { capability })
                    if capability == SERVICE_SESSION_CAPABILITY_V2),
                "{register:?}"
            );
            respond(&mut stream, &register, BOOT, Ok(registration())).await;
        });
        let client = make_client(socket, &root.join("intents"));
        client.register(&budget()).await.unwrap();
        server.await.unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn typed_send_returns_message_sent() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let register = incoming(&mut stream).await;
            respond(&mut stream, &register, BOOT, Ok(registration())).await;
            let send = incoming(&mut stream).await;
            let ServiceRequest::Operation(ServiceOperation::Send(request)) = &send.service else {
                panic!("expected a send frame: {send:?}");
            };
            assert_eq!(request.operation, OperationId::new("send-1"));
            let sent = ServiceMessageSent {
                summary: sent_summary(),
                author: ServiceAuthorId::new("graph"),
                recipient_count: 3,
                receipt_duration_millis: 60_000,
            };
            respond(
                &mut stream,
                &send,
                BOOT,
                Ok(ServiceResult::MessageSent(sent)),
            )
            .await;
        });
        let client = make_client(socket, &root.join("intents"));
        client.register(&budget()).await.unwrap();
        let (key, sent) = client
            .send(send_request("send-1"), &budget())
            .await
            .unwrap();
        assert_eq!(key, OperationId::new("send-1"));
        assert_eq!(sent.recipient_count, 3);
        assert_eq!(sent.summary.message.as_str(), "m-1");
        assert!(client.journal().pending().unwrap().is_empty());
        assert_eq!(client.journal().completed().unwrap(), vec![key]);
        server.await.unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn typed_reads_return_payloads_and_reject_mismatched_results() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let register = incoming(&mut stream).await;
            respond(&mut stream, &register, BOOT, Ok(registration())).await;
            let history = incoming(&mut stream).await;
            let mut page = empty_page();
            page.items.push(sent_summary());
            respond(
                &mut stream,
                &history,
                BOOT,
                Ok(ServiceResult::History(page)),
            )
            .await;
            // The wrong variant for a Receipts request.
            let receipts = incoming(&mut stream).await;
            respond(
                &mut stream,
                &receipts,
                BOOT,
                Ok(ServiceResult::History(empty_page())),
            )
            .await;
        });
        let client = make_client(socket, &root.join("intents"));
        client.register(&budget()).await.unwrap();
        let page = client.history(history_query(), &budget()).await.unwrap();
        assert_eq!(page.items, vec![sent_summary()]);
        let error = client
            .receipts(receipts_query(), &budget())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownOutcome);
        server.await.unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
