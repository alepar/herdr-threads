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
/// Ordinary send handlers anchor original ACK deadlines to committed send time.
pub const SEND_TIME_RECEIPTS_V1: &str = "messages.send_time_receipts_v1";

pub const SERVICE_SEND_V1: &str = "service.send_v1";
pub const HARNESS_EVIDENCE_V2: &str = "hook.harness_evidence_v2";
pub const HARNESS_EVIDENCE: &str = "hook.harness_evidence";
pub const HARNESS_HEALTH_V2: &str = "harness.health_v2";
pub const HARNESS_STATES: &str = "harness.states";
/// Same-native-session lifecycle/turn recovery through the existing C1 command.
pub const SAME_SESSION_CONTINUITY_V1: &str = "seat.continuity_same_session_v1";
/// Opt-in seat inspection exposes exact binding identity beside its liveness.
pub const SEAT_BINDING_DETAILS_V1: &str = "seat.binding_details_v1";
pub const SEAT_MANAGED_LAUNCH: &str = "seat.managed_launch";
pub const PARTICIPANT_LOCATIONS: &str = "participants.locations_v1";
/// `SeatsQuery.filter` and the seat directory entries (ht-xg88).
pub const SEATS_DIRECTORY_V1: &str = "seats.directory_v1";
/// `Command::WakeStatus`: per-seat wake blocks for the send warning and doctor.
pub const WAKE_STATUS_V1: &str = "seat.wake_status_v1";
/// `SyncCommand::PeerPrune` and `SyncCommand::PeerUnforget` (TRUST-POLICY
/// N16). The CLI sends them only to a daemon that advertises this, so an
/// older daemon gets one clear refusal instead of an unknown-variant error.
pub const PEER_PRUNE_V1: &str = "peer.prune_v1";
/// `SyncCommand::PeerEvents`, the link history.
pub const PEER_EVENTS_V1: &str = "peer.events_v1";

/// Canonical pair resolution and the independent SendMessage direct audience marker.
/// Advertise only after transactional pair, enrollment and audience guards are installed.
pub const ENSURE_CREATION_POLICY_V1: &str = "direct_threads.ensure_creation_policy_v1";
pub const DIRECT_THREADS_V1: &str = "thread.direct_v1";
/// Automatic handoff channel selection and exact canonical Ensure child attachment.
/// Advertise only with the complete automatic handoff deciding adapters.
pub const AUTOMATIC_HANDOFF_V1: &str = "handoff.automatic_v1";

/// Frozen profile/cwd projection and exact canonical bootstrap child retention.
/// Requires both guarded explicit-thread and automatic bootstrap implementations.
pub const HANDOFF_LAUNCH_DIRECTORIES_V1: &str = "handoff.launch_directories_v1";
/// Handoff plans may freeze a `summary`; their launch prompt is the one-line summary prompt.
pub const HANDOFF_SUMMARY_PROMPT_V1: &str = "handoff.summary_prompt_v1";

pub const THREAD_JOIN: &str = "thread.join_v1";

/// Advertise only once canonical invitation lifecycle handlers honor this policy.
pub const INVITATION_CONFIRMATION_V1: &str = "invitation.confirmation_v1";

pub const INVITATION_REJECT: &str = "invitation.reject_v1";

pub const LAZY_SEND: &str = "send.lazy_v1";
pub const INBOX_BATCH_V2: &str = "inbox.batch_v2";
pub const SEAT_WAKE_ON_V1: &str = "seat.wake_on_v1";
pub const MESSAGE_DELIVERY_MODES: &str = "messages.delivery_modes_v1";
/// Lazy rows in the attention digest (`AttentionDigestQuery.lazy`) and
/// `CompleteInboxDelivery.via = hook_context` (standard hook inbox pages).
pub const LAZY_HOOK_DELIVERY: &str = "hook.lazy_delivery_v1";
/// Complete native-hook bodies settle exact existing receipt obligations.
pub const HOOK_CONTEXT_RECEIPTS_V1: &str = "hook.context_receipts_v1";

/// Per-binding inbox stream: `InboxQuery.claim` selects the reading binding's cursors and
/// `Command::AdvanceCursors` moves them (ht-54r.7).
pub const BINDING_CURSORS: &str = "inbox.binding_cursors_v1";

/// Per-binding attention digest: `AttentionDigestQuery.claim` selects the reading binding's own
/// unread mail and notice frontier, and the digest token carries the binding (ht-54r.9).
pub const BINDING_DIGEST: &str = "attention.binding_digest_v1";

/// `AttentionDigestQuery.cli_hint`: the digest says whether the claimed binding lacks turn
/// delivery (`AttentionDigest.no_turn_delivery`), so a text command can name its pending count.
pub const CLI_PENDING_HINT: &str = "attention.cli_pending_hint_v1";

/// In-turn presentation: `AdvanceCursors.in_turn` (and its `invitations`) records, for a
/// top-level binding, the displayed messages whose receipt is still pending and the displayed
/// pending invitations, and the wake candidate owes that occupant no idle wake for them
/// (ht-705). Nothing is settled or accepted.
pub const IN_TURN_OFFER: &str = "inbox.in_turn_offer_v1";

/// End-of-turn delivery: `Command::TurnEndDue` answers whether an ending turn owes the caller's
/// seat a continuation (an item that would wake it idle). Without it a turn end never continues.
pub const TURN_END_DUE: &str = "turn_end.due_v1";

/// Thread-scoped lazy page: `InboxQuery.thread` on an `InboxBatchV2` first page selects only the
/// reader's unread lazy rows of that thread (ht-br0: a text `accept` shows the joined thread's
/// opening lazy mail in its own output).
pub const INBOX_THREAD_SCOPE: &str = "inbox.thread_scope_v1";

/// Pushed inbox page: `InboxQuery.pushed` on an `InboxBatchV2` first page applies the catch-up
/// hold (spec section 7) to the page, as the attention digest does (a hook's inbox check-in
/// offer). An explicit `inbox` read never sets it.
pub const INBOX_PUSHED_HOLD: &str = "inbox.pushed_hold_v1";

pub const INBOX_BATCH: &str = "inbox.batch_v1";
pub const ATTENTION_NOTICE_DELIVERY: &str = "attention.notice_delivery_v1";

/// The Claude mod watch connection (`protocol::watch`). Defined by the ht-j16.1
/// seam contract; its handler (`daemon::transport::watch_connection`) landed
/// with ht-j16.2.
pub const MOD_WATCH: &str = "mod.watch_v1";

/// Explicit Watch presentation opt-in and mode-bound Accepted lease.
/// Advertise only with the complete registration, registry and lease handlers.
pub const MOD_WATCH_PRESENTATION_V2: &str = "mod.watch_presentation_v2";

/// `AckModDelivered.row`: the mod reports the transcript row id the host stored for an `append`,
/// and the receipt's ACK observation records it as `row` (admission evidence, ht-admission-ack).
pub const MOD_ACK_ROW_V1: &str = "mod.ack_row_v1";

/// Hook-pull delivery (`protocol::delivery`, delivery-policy DP4). Defined by the
/// ht-poh.3.1 seam; its handlers (`service::delivery_pull`) landed with L4.
pub const DELIVERY_PULL: &str = "delivery.pull_v1";

/// `SendMessage.priority` and the priority-aware inbox/watch fields. Defined by the ht-yha.1 seam;
/// advertised with the send handler (ht-yha.3). Advertised for diagnostics; protocol-8 clients do not
/// probe it.
pub const SEND_PRIORITY_V1: &str = "send.priority_v1";
/// `Command::NudgeThreshold` / `Command::SetNudgeThreshold`. Defined by the ht-yha.1 seam;
/// advertised with its handlers (ht-yha.4). Advertised for diagnostics; protocol-8 clients do not
/// probe it.
pub const SEAT_NUDGE_THRESHOLD_V1: &str = "seat.nudge_threshold_v1";
/// 1:1 pairs auto-join both seats. Defined by the ht-yha.1 seam; advertised with the pair handler (ht-yha.6).
/// Advertised for diagnostics; protocol-8 clients do not probe it.
pub const DIRECT_AUTO_JOIN_V1: &str = "direct.auto_join_v1";

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
    SEND_TIME_RECEIPTS_V1,
    HARNESS_EVIDENCE,
    HARNESS_EVIDENCE_V2,
    HARNESS_STATES,
    HARNESS_HEALTH_V2,
    SEAT_MANAGED_LAUNCH,
    SEAT_BINDING_DETAILS_V1,
    SAME_SESSION_CONTINUITY_V1,
    INBOX_BATCH,
    INVITATION_REJECT,
    THREAD_JOIN,
    DIRECT_THREADS_V1,
    ENSURE_CREATION_POLICY_V1,
    PARTICIPANT_LOCATIONS,
    SEATS_DIRECTORY_V1,
    PICKER_DIRECTORY_V1,
    ATTENTION_NOTICE_DELIVERY,
    MESSAGE_DELIVERY_MODES,
    LAZY_SEND,
    INBOX_BATCH_V2,
    MOD_WATCH,
    MOD_WATCH_PRESENTATION_V2,
    MOD_ACK_ROW_V1,
    SEAT_WAKE_ON_V1,
    DELIVERY_PULL,
    LAZY_HOOK_DELIVERY,
    HOOK_CONTEXT_RECEIPTS_V1,
    DELIVERY_PRESENTATION_V2,
    BINDING_CURSORS,
    BINDING_DIGEST,
    IN_TURN_OFFER,
    INBOX_THREAD_SCOPE,
    SEND_PRIORITY_V1,
    SEAT_NUDGE_THRESHOLD_V1,
    DIRECT_AUTO_JOIN_V1,
    INBOX_PUSHED_HOLD,
    CLI_PENDING_HINT,
    TURN_END_DUE,
    WAKE_STATUS_V1,
    PEER_PRUNE_V1,
    PEER_EVENTS_V1,
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

/// Bounded ordered delivery through registered codecs and explicit presentation confirmation.
/// Consumers retain historical delivery when capability discovery omits this name.
pub const DELIVERY_PRESENTATION_V2: &str = "delivery.presentation_v2";
