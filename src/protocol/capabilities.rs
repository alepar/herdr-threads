//! Daemon capability discovery (ht-p03.43). Optional wire additions of this run are
//! gated by a named capability instead of a protocol_version bump. Discovery is a
//! separate request (`Command::Capabilities`) because the Health hello is
//! deny_unknown_fields in shipped CLIs; any refusal reads as "no capabilities".

use std::collections::BTreeSet;

/// Advertised only by an elected runtime with required scoped observer and canonical deciding handlers.
/// Only concrete runtimes enforcing immutable inspected-state recovery advertise this.
pub const BOOTSTRAP_INSPECTED_RECOVERY_V1: &str = "handoff.bootstrap_inspected_recovery_v1";

pub const BOOTSTRAP_GUARDED_RESOLUTION_V1: &str = "handoff.bootstrap_guarded_resolution_v1";

pub const PICKER_DIRECTORY_V1: &str = "picker.directory_v1";

pub const HISTORY_FULL_BODIES: &str = "history.full_bodies";
pub const HOOK_PARSE_FAILURE_REPORT: &str = "hook.parse_failure_report";
pub const SERVICE_SEND_V1: &str = "service.send_v1";
pub const HARNESS_EVIDENCE_V2: &str = "hook.harness_evidence_v2";
pub const HARNESS_EVIDENCE: &str = "hook.harness_evidence";
pub const HARNESS_HEALTH_V2: &str = "harness.health_v2";
pub const HARNESS_STATES: &str = "harness.states";
pub const SEAT_MANAGED_LAUNCH: &str = "seat.managed_launch";
pub const PARTICIPANT_LOCATIONS: &str = "participants.locations_v1";

pub const THREAD_JOIN: &str = "thread.join_v1";

pub const INVITATION_REJECT: &str = "invitation.reject_v1";

pub const LAZY_SEND: &str = "send.lazy_v1";
pub const INBOX_BATCH_V2: &str = "inbox.batch_v2";
pub const MESSAGE_DELIVERY_MODES: &str = "messages.delivery_modes_v1";

pub const INBOX_BATCH: &str = "inbox.batch_v1";
pub const ATTENTION_NOTICE_DELIVERY: &str = "attention.notice_delivery_v1";

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
    HARNESS_EVIDENCE_V2,
    HARNESS_STATES,
    HARNESS_HEALTH_V2,
    SEAT_MANAGED_LAUNCH,
    INBOX_BATCH,
    INVITATION_REJECT,
    THREAD_JOIN,
    PARTICIPANT_LOCATIONS,
    PICKER_DIRECTORY_V1,
    ATTENTION_NOTICE_DELIVERY,
    MESSAGE_DELIVERY_MODES,
    LAZY_SEND,
    INBOX_BATCH_V2,
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
