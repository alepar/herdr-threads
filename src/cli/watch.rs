//! Hidden `watch` and `watch ack` subcommands: the Claude mod's delivery child.
//! The contract (argv, env, exit codes, JSON lines) is in
//! [`crate::protocol::watch`].

use super::RunError;
use crate::harness::context::Harness;
use crate::protocol::{
    ids::MessageId,
    results::ApiError,
    watch::{
        ModDeliveryVia, WATCH_EXIT_STOP, WatchItem, WatchLine, WatchStatus, WatchStatusReason,
        WatchStatusState,
    },
};
use std::io::Write;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRequest {
    pub harness: Harness,
    pub session: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchAckRequest {
    pub session: String,
    pub via: ModDeliveryVia,
    pub messages: Vec<MessageId>,
}

/// Stub from the ht-j16.1 seam contract; ht-j16.5 implements. Prints one
/// status line and exits 3 (`unsupported`) without contacting a daemon.
pub fn run_watch<W: Write>(_request: &WatchRequest, writer: &mut W) -> Result<(), RunError> {
    let line = WatchLine::new(
        "status:0",
        WatchItem::Status(WatchStatus {
            state: WatchStatusState::Refused,
            reason: Some(WatchStatusReason::Unsupported),
            exit: Some(WATCH_EXIT_STOP),
        }),
    );
    let text = serde_json::to_string(&line).map_err(|error| {
        RunError::Api(ApiError::invalid_request(format!(
            "status encoding failed: {error}"
        )))
    })?;
    writer.write_all(text.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Err(RunError::Exit(WATCH_EXIT_STOP))
}

/// Stub from the ht-j16.1 seam contract; ht-j16.5 implements. Writes no
/// lines, so the mod treats every id as retryable.
pub fn run_ack<W: Write>(_request: &WatchAckRequest, _writer: &mut W) -> Result<(), RunError> {
    Err(ApiError::unsupported("watch ack is not implemented in this build").into())
}

#[cfg(test)]
#[path = "../../tests/cli/watch.rs"]
mod tests;
