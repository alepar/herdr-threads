//! Send-priority resolution (message-priority spec D3, D4): the one place the CLI turns send flags
//! into a priority; send, send --seat, human send and handoff all go through here.

use crate::protocol::{priority::DeliveryPriority, results::ApiError};

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

#[cfg(test)]
#[path = "../../tests/cli/send_priority.rs"]
mod tests;
