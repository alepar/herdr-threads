//! Daemon capability discovery (ht-p03.43). Optional wire additions of this run are
//! gated by a named capability instead of a protocol_version bump. Discovery is a
//! separate request (`Command::Capabilities`) because the Health hello is
//! deny_unknown_fields in shipped CLIs; any refusal reads as "no capabilities".

use std::collections::BTreeSet;

pub const HISTORY_FULL_BODIES: &str = "history.full_bodies";
pub const HOOK_PARSE_FAILURE_REPORT: &str = "hook.parse_failure_report";
pub const HARNESS_EVIDENCE: &str = "hook.harness_evidence";
pub const HARNESS_STATES: &str = "harness.states";

/// Everything this daemon build serves. A capability is listed only once its
/// handler has landed (ht-p03.105): `HISTORY_FULL_BODIES` landed with
/// ht-p03.12.8; `HOOK_PARSE_FAILURE_REPORT` landed with ht-p03.23;
/// `HARNESS_EVIDENCE` landed with ht-xoc.4; `HARNESS_STATES` landed with
/// ht-xoc.5. Each entry
/// has a probe arm in `every_advertised_capability_has_a_handler`.
pub const ADVERTISED: &[&str] = &[
    HISTORY_FULL_BODIES,
    HOOK_PARSE_FAILURE_REPORT,
    HARNESS_EVIDENCE,
    HARNESS_STATES,
];

/// The capability set a daemon advertised to this client session. The empty set
/// is also what an older daemon, which cannot answer the request, reads as.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities(BTreeSet<String>);

impl Capabilities {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn from_list(list: impl IntoIterator<Item = String>) -> Self {
        Self(list.into_iter().collect())
    }

    pub fn supports(&self, name: &str) -> bool {
        self.0.contains(name)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
#[path = "../../tests/protocol/capabilities.rs"]
mod tests;
