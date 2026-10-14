//! Send-priority resolution and the new-client/old-daemon wire shape (message-priority spec D3, D4,
//! §Compatibility). The one place the CLI turns send flags into a priority and a priority into a
//! request shape; send, send --seat, human send and handoff all go through here.

use crate::ports::LocalClient;
use crate::protocol::{
    capabilities::SEND_PRIORITY_V1,
    commands::{Command, DeliveryMode},
    priority::DeliveryPriority,
    results::{ApiError, CommandResult, ErrorCode},
    time::CallBudget,
};

/// The CLI's send-priority flags, before any body is read.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SendPriorityFlags {
    pub priority: Option<DeliveryPriority>,
    pub nudge: bool,
    pub explicit_ack: bool,
    pub deadline: bool,
}

/// D3/D4: `--nudge` = p0; explicit ACK recipients force p0 and refuse an explicit p > 0; `--deadline`
/// needs explicit ACK recipients; default p1. Shared by send, send --seat, human send and handoff.
pub(crate) fn resolve(flags: SendPriorityFlags) -> Result<DeliveryPriority, ApiError> {
    if flags.deadline && !flags.explicit_ack {
        return Err(ApiError::invalid_request(
            "--deadline requires --require-ack recipients",
        ));
    }
    if flags.explicit_ack {
        if flags.priority.is_some_and(|p| p.get() > 0) {
            return Err(ApiError::invalid_request(
                "--require-ack implies priority 0; drop --priority",
            ));
        }
        return Ok(DeliveryPriority::p0());
    }
    if flags.nudge {
        return Ok(DeliveryPriority::p0());
    }
    Ok(flags.priority.unwrap_or_default())
}

/// The request shape a send takes, decided once before the intent is recorded so a retry replays
/// exactly what was first sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendShape {
    /// `send.priority_v1`: `priority` carried, `delivery_mode` omitted.
    Priority(DeliveryPriority),
    /// An older daemon: the two-valued mode; `lost` is the priority (> 1) it cannot express.
    Legacy {
        mode: DeliveryMode,
        lost: Option<DeliveryPriority>,
    },
}

impl SendShape {
    /// The `(delivery_mode, priority)` pair of a `SendMessage` request.
    pub(crate) fn wire(self) -> (DeliveryMode, Option<DeliveryPriority>) {
        match self {
            Self::Priority(p) => (DeliveryMode::Ordinary, Some(p)),
            Self::Legacy { mode, .. } => (mode, None),
        }
    }
}

/// The one stderr line (about 80 bytes) when the daemon cannot express the priority.
pub(crate) fn lossy_notice(lost: DeliveryPriority) -> String {
    format!(
        "herdr-threads: this daemon lacks send.priority_v1; priority {} sent as lazy (p1)",
        lost.get()
    )
}

pub(crate) fn shape(priority: DeliveryPriority, priority_v1: bool) -> SendShape {
    if priority_v1 {
        return SendShape::Priority(priority);
    }
    let legacy = priority.to_mode();
    SendShape::Legacy {
        mode: legacy.mode,
        lost: legacy.lossy.then_some(priority),
    }
}

/// Whether the daemon advertises `send.priority_v1`. `Ok(false)` only when the daemon answered and
/// lacks it; a failed probe (busy, timeout, transport loss) is an `Err`, never a silent downgrade
/// of the send to the legacy shape. `supports_capability` reads a failed call as "no capabilities"
/// (and caches that), so a miss there is confirmed with one fresh capabilities call.
pub(crate) fn supports<C: LocalClient + ?Sized>(
    client: &C,
    budget: &CallBudget,
) -> Result<bool, ApiError> {
    if client.supports_capability(SEND_PRIORITY_V1, budget) {
        return Ok(true);
    }
    match client.call(Command::Capabilities, budget) {
        Ok(CommandResult::Capabilities(list)) => {
            Ok(list.capabilities.iter().any(|c| c == SEND_PRIORITY_V1))
        }
        Ok(_) => Ok(false),
        Err(e) if matches!(e.code, ErrorCode::Unsupported | ErrorCode::InvalidRequest) => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
#[path = "../../tests/cli/send_priority.rs"]
mod tests;
