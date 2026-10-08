//! Thin local Unix socket client. A request is sent once; transport never retries.

use crate::{
    daemon::transport::{MAX_FRAME_BYTES, encode_json, read_frame},
    ports::LocalClient,
    protocol::{
        capabilities::Capabilities,
        commands::Command,
        output::OutputSpec,
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Clock},
        wire::{PROTOCOL_VERSION, WireRequest, WireResponse},
    },
};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    net::UnixStream,
};
use uuid::Uuid;

pub struct LocalSocketClient {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    expected_instance: Uuid,
    expected_boot: Option<Uuid>,
    capabilities: OnceLock<Capabilities>,
}
impl LocalSocketClient {
    pub fn new(
        path: PathBuf,
        clock: Arc<dyn Clock>,
        expected_instance: Uuid,
        expected_boot: Option<Uuid>,
    ) -> Self {
        Self {
            path,
            clock,
            expected_instance,
            expected_boot,
            capabilities: OnceLock::new(),
        }
    }

    /// The capabilities this daemon advertises, asked once per session (ht-p03.43).
    /// Any refusal or transport error (an older daemon cannot decode the request and
    /// closes, or answers an error) reads as no capabilities, never as a failure.
    pub fn capabilities(&self, budget: &CallBudget) -> Capabilities {
        self.capabilities
            .get_or_init(|| match self.call(Command::Capabilities, budget) {
                Ok(CommandResult::Capabilities(list)) => Capabilities::from_list(list.capabilities),
                _ => Capabilities::none(),
            })
            .clone()
    }

    fn error(code: ErrorCode, detail: &str) -> ApiError {
        ApiError::new(code, detail)
    }
    fn remaining(&self, budget: &CallBudget) -> Result<Duration, ApiError> {
        if budget.cancellation.is_cancelled() {
            return Err(Self::error(ErrorCode::Cancelled, "request cancelled"));
        }
        let ms = budget
            .deadline
            .0
            .saturating_sub(self.clock.monotonic_now().0);
        if ms == 0 {
            return Err(Self::error(
                ErrorCode::DeadlineExceeded,
                "request deadline elapsed",
            ));
        }
        Ok(Duration::from_millis(ms))
    }

    /// Each `write` reports its completed byte count. Tokio's single-write
    /// future is cancellation-safe, so an interrupted write cannot hide a
    /// completed frame from this progress count.
    pub(crate) async fn write_request<W: AsyncWrite + Unpin>(
        &self,
        socket: &mut W,
        body: &[u8],
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        if body.is_empty() || body.len() > MAX_FRAME_BYTES {
            return Err(Self::error(
                ErrorCode::InvalidRequest,
                "invalid request frame length",
            ));
        }
        let remaining = self.remaining(budget)?;
        let deadline = tokio::time::Instant::now() + remaining;
        let prefix = (body.len() as u32).to_be_bytes();
        for mut part in [&prefix[..], body] {
            while !part.is_empty() {
                let wrote = tokio::select! {
                    result = tokio::time::timeout_at(deadline, socket.write(part)) => {
                        result
                            .map_err(|_| Self::error(ErrorCode::HostUnavailable, "incomplete request frame"))?
                            .map_err(|_| Self::error(ErrorCode::HostUnavailable, "incomplete request frame"))?
                    }
                    _ = budget.cancellation.cancelled() => return Err(Self::error(ErrorCode::HostUnavailable, "incomplete request frame")),
                };
                if wrote == 0 {
                    return Err(Self::error(
                        ErrorCode::HostUnavailable,
                        "incomplete request frame",
                    ));
                }
                part = &part[wrote..];
            }
        }
        Ok(())
    }

    pub async fn call_async(
        &self,
        command: Command,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call_async_selected(command, None, budget).await
    }

    pub async fn call_async_with_output(
        &self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call_async_selected(command, Some(output), budget)
            .await
    }

    async fn call_async_selected(
        &self,
        command: Command,
        output: Option<&OutputSpec>,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.exchange_selected(command, output, budget)
            .await
            .and_then(|result| result)
    }

    /// Outer error: the request was not sent, or its outcome is unknown.
    /// Inner result: the daemon's correlated, definitive answer.
    async fn exchange_selected(
        &self,
        command: Command,
        output: Option<&OutputSpec>,
        budget: &CallBudget,
    ) -> Result<Result<CommandResult, ApiError>, ApiError> {
        if let Some(output) = output {
            output
                .validate()
                .map_err(|detail| Self::error(ErrorCode::InvalidRequest, detail))?;
        }
        let request = WireRequest {
            version: PROTOCOL_VERSION,
            request_id: uuid::Uuid::new_v4().to_string(),
            expected_instance: self.expected_instance.to_string(),
            expected_boot: self.expected_boot.map(|boot| boot.to_string()),
            output: output.cloned(),
            command,
        };
        let body = encode_json(&request)
            .map_err(|_| Self::error(ErrorCode::InvalidRequest, "request exceeds frame limit"))?;
        let connect_budget = self.remaining(budget)?.min(Duration::from_secs(2));
        let mut socket = tokio::select! {
            result = tokio::time::timeout(connect_budget, UnixStream::connect(&self.path)) => {
                result.map_err(|_| Self::error(ErrorCode::HostUnavailable, "daemon connect timed out"))?
                    .map_err(|error| super::connect_error(&error, &self.path))?
            }
            _ = budget.cancellation.cancelled() => return Err(Self::error(ErrorCode::Cancelled, "request cancelled before connect")),
        };
        self.write_request(&mut socket, &body, budget).await?;
        let remaining = self.remaining(budget).map_err(|_| {
            Self::error(
                ErrorCode::UnknownOutcome,
                "unknown outcome after request submission",
            )
        })?;
        let deadline = tokio::time::Instant::now() + remaining;
        let read = read_frame(&mut socket);
        tokio::pin!(read);
        let response_bytes = tokio::select! {
            result = &mut read => result.map_err(|_| Self::error(ErrorCode::UnknownOutcome, "unknown outcome after request submission"))?,
            _ = tokio::time::sleep_until(deadline) => return Err(Self::error(ErrorCode::UnknownOutcome, "unknown outcome after request submission")),
            _ = budget.cancellation.cancelled() => return Err(Self::error(ErrorCode::UnknownOutcome, "unknown outcome after request submission")),
        };
        let response: WireResponse = serde_json::from_slice(&response_bytes).map_err(|_| {
            Self::error(
                ErrorCode::UnknownOutcome,
                "unknown outcome after request submission",
            )
        })?;
        if let Err(error) = &response.result
            && error.code == ErrorCode::DaemonBootChanged
            && response.correlates_to(&request, None)
        {
            // The daemon refused before dispatch: a definite rejection.
            return Ok(response.result);
        }
        let expected_boot = self.expected_boot.map(|boot| boot.to_string());
        let valid_boot = Uuid::parse_str(&response.daemon_boot)
            .is_ok_and(|boot| boot.to_string() == response.daemon_boot);
        if !valid_boot || !response.correlates_to(&request, expected_boot.as_deref()) {
            return Err(Self::error(
                ErrorCode::UnknownOutcome,
                "unknown outcome: response identity mismatch",
            ));
        }
        Ok(response.result)
    }

    /// Like `call_with_output`, but keeps a correlated daemon rejection
    /// (inner `Err`) distinct from a transport failure or unknown outcome.
    pub fn call_with_output_definitive(
        &self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<Result<CommandResult, ApiError>, ApiError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Self::error(ErrorCode::HostUnavailable, "client runtime unavailable"))?;
        runtime.block_on(self.exchange_selected(command, Some(output), budget))
    }

    /// Like `call`, but keeps a correlated daemon rejection distinct from a
    /// transport failure or unknown outcome.
    pub fn call_definitive(
        &self,
        command: Command,
        budget: &CallBudget,
    ) -> Result<Result<CommandResult, ApiError>, ApiError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Self::error(ErrorCode::HostUnavailable, "client runtime unavailable"))?;
        runtime.block_on(self.exchange_selected(command, None, budget))
    }

    pub fn call_with_output(
        &self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Self::error(ErrorCode::HostUnavailable, "client runtime unavailable"))?;
        runtime.block_on(self.call_async_with_output(command, output, budget))
    }
}
impl LocalClient for LocalSocketClient {
    fn supports_capability(&self, name: &str, budget: &CallBudget) -> bool {
        self.capabilities(budget).supports(name)
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Self::error(ErrorCode::HostUnavailable, "client runtime unavailable"))?;
        runtime.block_on(self.call_async(command, budget))
    }
    fn call_with_output(
        &self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        LocalSocketClient::call_with_output(self, command, output, budget)
    }
    fn call_definitive(
        &self,
        command: Command,
        budget: &CallBudget,
    ) -> Result<Result<CommandResult, ApiError>, ApiError> {
        LocalSocketClient::call_definitive(self, command, budget)
    }
}
