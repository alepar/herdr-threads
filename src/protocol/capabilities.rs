//! Daemon capability discovery (ht-p03.43). Optional wire additions of this run are
//! gated by a named capability instead of a protocol_version bump. Discovery is a
//! separate request (`Command::Capabilities`) because the Health hello is
//! deny_unknown_fields in shipped CLIs; any refusal reads as "no capabilities".

use std::collections::BTreeSet;

pub const PICKER_DIRECTORY_V1: &str = "picker.directory_v1";

pub const HISTORY_FULL_BODIES: &str = "history.full_bodies";
pub const HOOK_PARSE_FAILURE_REPORT: &str = "hook.parse_failure_report";
pub const SERVICE_SEND_V1: &str = "service.send_v1";
pub const HARNESS_EVIDENCE: &str = "hook.harness_evidence";
pub const HARNESS_STATES: &str = "harness.states";
pub const SEAT_MANAGED_LAUNCH: &str = "seat.managed_launch";
pub const PARTICIPANT_LOCATIONS: &str = "participants.locations_v1";

pub const INVITATION_REJECT: &str = "invitation.reject_v1";

pub const INBOX_BATCH: &str = "inbox.batch_v1";

/// Everything this daemon build serves. A capability is listed only once its
/// handler has landed (ht-p03.105): `HISTORY_FULL_BODIES` landed with
/// ht-p03.12.8; `HOOK_PARSE_FAILURE_REPORT` landed with ht-p03.23;
/// `SERVICE_SEND_V1` landed with ht-5nb.2/.3; `HARNESS_EVIDENCE` landed with
/// ht-xoc.4; `HARNESS_STATES` landed with ht-xoc.5; `SEAT_MANAGED_LAUNCH`
/// landed with ht-5n6; `INBOX_BATCH` landed with compact inbox display ACK.
/// Each entry
/// has a probe arm in `every_advertised_capability_has_a_handler`.
pub const ADVERTISED: &[&str] = &[
    HISTORY_FULL_BODIES,
    HOOK_PARSE_FAILURE_REPORT,
    SERVICE_SEND_V1,
    HARNESS_EVIDENCE,
    HARNESS_STATES,
    SEAT_MANAGED_LAUNCH,
    INBOX_BATCH,
    INVITATION_REJECT,
    PARTICIPANT_LOCATIONS,
    PICKER_DIRECTORY_V1,
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
