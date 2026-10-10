//! Digest suppression per delivery mechanism (delivery-policy DP7). This is
//! presentation, not authority, so a compiled registry lookup is enough.

use crate::harness::adapter::{DeliveryMechanism, DeliveryPolicy};

/// Where in a harness session a check-in runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    /// SessionStart-like: the lifecycle check-in.
    Lifecycle,
    /// A tool boundary inside a turn.
    Tool,
}

/// Whether the attention digest (line, ready commands, mark seed) is left out
/// of a check-in. `channel_live` is the daemon's per-seat "a mod channel is
/// live" flag. A hook-pull seat keeps the lifecycle digest (the SessionStart
/// context shows non-body attention there) and omits it at tool boundaries,
/// where `PreInvocation` pulls deliver instead; every other seat omits the
/// digest exactly while its channel is live.
pub fn omit_digest(
    policy: Option<&dyn DeliveryPolicy>,
    channel_live: bool,
    boundary: Boundary,
) -> bool {
    match policy.map(|policy| policy.mechanism()) {
        Some(DeliveryMechanism::HookPull) => matches!(boundary, Boundary::Tool),
        _ => channel_live,
    }
}

#[cfg(test)]
#[path = "../../tests/delivery/routing.rs"]
mod tests;
