use crate::protocol::service::{EventAuthor, RequiredMembership};
use crate::protocol::{ids::*, pagination::Page, time::UtcMillis};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CommandResult {
    Health(Health),
    StopAccepted(StopAccepted),
    ServiceInspection(ServiceConnectionInspection),
    ServiceDisconnected(ServiceDisconnectResult),
    Directory(Page<ThreadSummary>),
    Seats(Page<SeatSummary>),
    SeatInspect(SeatInspection),
    Inbox(Page<InboxItem>),
    Warnings(Page<WarningRef>),
    Thread(ThreadDetails),
    History(Page<MessageSummary>),
    Participants(Page<Participant>),
    Recipients(Page<Recipient>),
    /// Warning recipients have no receipt or ACK obligation.
    WarningRecipients(Page<WarningRecipient>),
    DeliveryInspect(DeliveryInspection),
    PendingReceipts(Page<PendingReceipt>),
    AttentionDigest(crate::protocol::attention::AttentionDigest),
    LocalIntents(Page<LocalIntent>),
    Search(SearchPage),
    Message(MessageDetails),
    Diagnostics(Page<Diagnostic>),
    OperationStatus(OperationStatus),
    RetirementJobs(Page<RetirementStatus>),
    SeatResolved(SeatId),
    CheckedIn(CheckInResult),
    /// Local presentation of an immutable completed CheckIn fragment.
    #[serde(skip_deserializing)]
    CachedCheckInPage(crate::harness::cache::CachedCheckInPage),
    ThreadCreated(ThreadId),
    Invitation(InvitationId),
    AlreadyJoined(AlreadyJoined),
    Accepted(InvitationId),
    RequiredAccepted(RequiredMembership),
    MessageSent(MessageId),
    Acknowledged(AckResult),
    Left(ThreadId),
    TopicChanged(ThreadId),
    Archived(ThreadId),
    Reopened(ThreadId),
    OperatorRebound(SeatId),
    OperatorFreshSeat(SeatId),
    OperatorInvited(InvitationId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopAccepted {
    /// The boot that accepted shutdown; draining and owner exit follow.
    pub boot_id: String,
}

/// A connection observation. It does not attest native agent liveness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConnectionInspection {
    pub instance: String,
    pub daemon_boot: String,
    pub connected: bool,
    /// Most recently registered generation in this boot, including after EOF.
    pub connection_generation: Option<u64>,
    pub registered_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "error", rename_all = "snake_case")]
pub enum ServiceRecoveryAudit {
    Persisted,
    Failed(ApiError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDisconnectResult {
    pub instance: String,
    pub daemon_boot: String,
    pub connection_generation: u64,
    pub disconnected: bool,
    pub audit: ServiceRecoveryAudit,
}

/// An invite was replayed for a seat that is already joined. No invitation
/// episode, deadline, or acceptance was created by this decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlreadyJoined {
    pub thread: ThreadId,
    pub seat: SeatId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unsupported,
    InvalidRequest,
    UnknownWireVersion,
    DaemonVersionMismatch,
    InstanceMismatch,
    /// The request named a daemon boot that is no longer running; nothing was
    /// dispatched. Re-read the descriptor and retry.
    DaemonBootChanged,
    CursorStale,
    InvalidCursor,
    InvalidBudget,
    ReadBudgetExhausted,
    SequenceExhausted,
    Unauthorized,
    CallerUnverified,
    PermitExpired,
    StaleHostObservation,
    TargetUnresolved,
    TargetUnsafe,
    TargetAlreadyOwned,
    ThreadNotOrphaned,
    Archived,
    NotFound,
    OperationPayloadMismatch,
    StoreBusy,
    StoreCorrupt,
    /// The database cannot grow (SQLITE_FULL): free disk space, then retry.
    /// Nothing was accepted; stored history is intact.
    StoreFull,
    IncompatibleSchema,
    HostUnavailable,
    UnknownOutcome,
    MissingHook,
    UnsupportedHarness,
    Cancelled,
    DeadlineExceeded,
    Conflict,
    ServiceBusy,
    ServiceNotRegistered,
    StaleServiceGeneration,
    IncompatibleOwnership,
    RequiredInvitationNeedsManagedThread,
    MembershipRequired,
    StaleRequirementAcceptance,
    /// Client-local, never sent by a daemon: the operating system refused the
    /// connect to the daemon socket (EPERM/EACCES), typically a harness
    /// sandbox that does not allow that Unix socket. Restarting the daemon
    /// cannot fix it; the sandbox must allow the stable socket pathname.
    TransportDenied,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiError {
    pub code: ErrorCode,
    pub detail: String,
    pub restart_argv: Option<Vec<String>>,
    #[serde(default)]
    pub required_minimum_bytes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Health {
    pub software_version: String,
    pub protocol_version: u16,
    pub instance_id: String,
    pub boot_id: String,
    pub state: HealthState,
    pub database: HealthComponent,
    pub schema: HealthComponent,
    pub host: HostHealth,
    pub last_reconciliation_at: Option<UtcMillis>,
    pub last_scheduler_tick_at: Option<UtcMillis>,
    pub harness: HarnessHealth,
    /// `None` means the bounded producer has no reliable scalar, never zero.
    pub unresolved_seats: Option<u64>,
    pub held_targets: Option<u64>,
    pub retirement_pending: Option<u64>,
    pub retirement_degraded: Option<u64>,
    pub host_timeouts: Option<u64>,
    pub queue_rejections: Option<u64>,
    pub queue_depth: Option<u64>,
    /// Resolved per-instance timing values. None means this bounded health
    /// producer has not supplied the elected service configuration.
    pub settings: Option<HealthSettings>,
    /// Problems that need attention (a `degraded` state names at least one).
    /// A schema-matched, live-unverified Codex admission is listed here even
    /// while the state is `healthy`.
    pub limitations: Vec<String>,
    /// Informational facts about the designed operating mode (cooperative
    /// receipts and wake, a harness absent from the daemon's `PATH`). Never
    /// a reason for `degraded`. Absent from an older daemon's Health.
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthSettings {
    pub invitation_default_ms: u64,
    pub receipt_default_ms: u64,
    pub minimum_wake_delay_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    Unknown,
    Ready,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthComponent {
    pub state: ComponentState,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Unknown,
    Unsupported,
    Supported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostHealth {
    pub reachability: ComponentState,
    pub version: Option<String>,
    pub current_execution: CapabilityState,
    pub coherent_enumeration: CapabilityState,
    pub safe_prompt: CapabilityState,
    pub receipt_registration: CapabilityState,
}

/// One harness as the daemon observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessState {
    /// Not observed yet.
    Unknown,
    /// Not usable here: no executable on the daemon's `PATH` (a note, not a
    /// problem) or an installed version no recipe admits (a limitation).
    Unsupported,
    /// The designed mode: the installed version is admitted by a recipe and
    /// model-issued accept/ACK is recorded as `cooperative_top_level`
    /// provenance, never as a native-verified receipt.
    Cooperative,
    /// Native-verified model receipt (no recipe declares it in this build).
    Supported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessHealth {
    pub codex: HarnessState,
    pub claude: HarnessState,
}

impl Health {
    /// Honest starting point for a reachable daemon before bounded providers
    /// have supplied their observations. Callers must populate proven fields.
    pub fn unknown(
        instance_id: String,
        boot_id: String,
        software_version: String,
        protocol_version: u16,
    ) -> Self {
        Self {
            software_version,
            protocol_version,
            instance_id,
            boot_id,
            state: HealthState::Degraded,
            database: HealthComponent {
                state: ComponentState::Unknown,
                detail: None,
            },
            schema: HealthComponent {
                state: ComponentState::Unknown,
                detail: None,
            },
            host: HostHealth {
                reachability: ComponentState::Unknown,
                version: None,
                current_execution: CapabilityState::Unknown,
                coherent_enumeration: CapabilityState::Unknown,
                safe_prompt: CapabilityState::Unknown,
                receipt_registration: CapabilityState::Unknown,
            },
            last_reconciliation_at: None,
            last_scheduler_tick_at: None,
            harness: HarnessHealth {
                codex: HarnessState::Unknown,
                claude: HarnessState::Unknown,
            },
            unresolved_seats: None,
            held_targets: None,
            retirement_pending: None,
            retirement_degraded: None,
            host_timeouts: None,
            queue_rejections: None,
            queue_depth: None,
            settings: None,
            limitations: Vec::new(),
            notes: Vec::new(),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if uuid::Uuid::parse_str(&self.instance_id)
            .map_or(true, |id| id.to_string() != self.instance_id)
            || uuid::Uuid::parse_str(&self.boot_id)
                .map_or(true, |id| id.to_string() != self.boot_id)
        {
            return Err("invalid health identity");
        }
        if self.software_version.len() > 128
            || self.software_version.is_empty()
            || self.host.version.as_ref().is_some_and(|s| s.len() > 128)
            || [self.database.detail.as_ref(), self.schema.detail.as_ref()]
                .iter()
                .flatten()
                .any(|s| s.len() > 256)
            || self.limitations.len() > 16
            || self.limitations.iter().any(|s| s.len() > 256)
            || self.notes.len() > 16
            || self.notes.iter().any(|s| s.len() > 256)
        {
            return Err("unbounded health detail");
        }
        if self.state == HealthState::Healthy
            && (self.database.state != ComponentState::Ready
                || self.schema.state != ComponentState::Ready
                || (self.host.current_execution != CapabilityState::Supported
                    && self.host.safe_prompt != CapabilityState::Supported)
                || self.host.coherent_enumeration != CapabilityState::Supported)
        {
            return Err("health claims unproven readiness");
        }
        if self.settings.as_ref().is_some_and(|settings| {
            [
                settings.invitation_default_ms,
                settings.receipt_default_ms,
                settings.minimum_wake_delay_ms,
            ]
            .iter()
            .any(|value| *value == 0 || *value > i64::MAX as u64)
        }) {
            return Err("invalid health settings");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadSummary {
    pub thread: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_owner: Option<ServiceAuthorId>,
    pub topic_data: String,
    pub topic_omitted: bool,
    pub topic_detail_argv: Option<Vec<String>>,
    pub archived: bool,
    pub orphaned: bool,
    pub message_count: u64,
    pub created_at: UtcMillis,
    pub ordinary_count: u64,
    pub system_count: u64,
    pub joined_count: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuityStatus {
    Resolved,
    Unresolved,
    Retired,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatSummary {
    pub seat: SeatId,
    pub continuity: ContinuityStatus,
    pub target: Option<HostTargetId>,
    pub generation: u64,
    pub created_at: UtcMillis,
    pub retired_at: Option<UtcMillis>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingStatus {
    pub state: ContinuityStatus,
    pub target: Option<HostTargetId>,
    pub detail_argv: Option<Vec<String>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldSummary {
    pub target: HostTargetId,
    pub reason_data: String,
    pub detail_argv: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatInspection {
    pub summary: SeatSummary,
    pub mapping: MappingStatus,
    pub hold: Option<HoldSummary>,
    pub retirement: Option<RetirementStatus>,
    pub history: Page<SeatHistoryItem>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum SeatHistoryItem {
    Binding(BindingHistory),
    Repair(RepairHistory),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingHistory {
    pub ordinal: u64,
    pub generation: u64,
    pub target: HostTargetId,
    pub terminal: Option<TerminalId>,
    pub incarnation: Option<String>,
    pub host_boot: HostBootId,
    pub host_epoch: u64,
    pub native_session: NativeSessionId,
    pub execution: ExecutionId,
    pub observed_at: UtcMillis,
    pub registered_at: Option<UtcMillis>,
    pub ended_at: Option<UtcMillis>,
    pub provenance: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairHistory {
    pub ordinal: u64,
    pub target: HostTargetId,
    pub decision_kind: String,
    pub decided_at: UtcMillis,
    pub host_boot: HostBootId,
    pub host_epoch: u64,
    pub generation: u64,
    pub operator_label: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboxItem {
    pub thread: ThreadId,
    /// Pending invitations of the seat in this thread, saturated at
    /// `MAX_PENDING_WARNING_COUNT` (bounded-walk invariant).
    pub invitations: u64,
    /// True when more invitations may be pending than `invitations` says.
    #[serde(default, skip_serializing_if = "is_false")]
    pub invitations_has_more: bool,
    /// Pending receipts addressed to the seat in this thread, saturated at
    /// `MAX_PENDING_WARNING_COUNT`.
    pub pending_receipts: u64,
    /// True when more receipts may be pending than `pending_receipts` says.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pending_receipts_has_more: bool,
    /// Pending (actionable) warnings addressed to the seat in this thread,
    /// capped at `MAX_PENDING_WARNING_COUNT`; not a historical total. Settled
    /// warnings stay readable through `warnings` and thread history.
    pub warnings: u64,
    /// True when more than `MAX_PENDING_WARNING_COUNT` warnings may be
    /// pending for the seat in this thread.
    #[serde(default)]
    pub warnings_has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_requirement: Option<RequiredMembership>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Cap on every pending count carried by the digest, check-in and inbox. Each
/// count comes from per-source `LIMIT cap+1` walks, so a set `has_more` flag
/// means "more than the counted items may be pending" (bounded-walk
/// invariant, wave-2 fix1 (a)); the
/// complete warning history stays available through the paginated, read-only
/// `warnings` continuation.
pub const MAX_PENDING_WARNING_COUNT: u64 = 1_000;
/// Most programmatic service notices one check-in offer carries (and so
/// settles): the offered page is capped per request, so settlement is O(cap)
/// whatever the undelivered backlog (wave-2 fix2 root decision (a)). The page
/// is further trimmed to the offer's selected output bound.
pub const MAX_NOTICE_PAGE_ITEMS: usize = 16;

/// The page of programmatic service warn notices a check-in offer carries to
/// the seat's current occupant, oldest first above that occupant's offered
/// frontier. A committed offer settles exactly these notices, whether or not
/// its output later reaches the agent (notices are informational, not
/// obligations); `has_more` says more unoffered notices remain for later
/// offers. Every notice, settled or not, stays in the paginated `warnings`
/// history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeOffer {
    pub items: Vec<WarningRef>,
    pub has_more: bool,
}

impl NoticeOffer {
    /// Nothing carried and nothing more to carry.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && !self.has_more
    }
    /// Compact fixed-shape line naming the carried notices (service-generated
    /// identifiers only), shown beside the digest summary; `None` when the
    /// offer carried no notice.
    pub fn summary(&self) -> Option<String> {
        if self.items.is_empty() {
            return None;
        }
        let items: Vec<String> = self
            .items
            .iter()
            .map(|notice| format!("{}@{}", notice.warning.as_str(), notice.thread.as_str()))
            .collect();
        Some(format!(
            "offered notices: {} [{}]{}",
            items.len(),
            items.join(", "),
            if self.has_more { " +more" } else { "" }
        ))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarningRef {
    pub warning: MessageId,
    pub thread: ThreadId,
    /// Immutable per-thread history position.
    pub sequence: u64,
    /// Global decision generation used for recipient-at-E and offer frontier.
    pub event_seq: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadDetails {
    pub summary: ThreadSummary,
    pub goal_data: String,
    pub created_at: UtcMillis,
    pub participant_count: u64,
    pub participants: Page<Participant>,
    pub pending_receipt_count: u64,
    pub pending_receipts_argv: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSummary {
    pub message: MessageId,
    pub thread: ThreadId,
    pub author: Option<SeatId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_author: Option<EventAuthor>,
    pub kind: MessageKind,
    pub sequence: u64,
    pub created_at: UtcMillis,
    pub actor_label: Option<String>,
    pub preview_data: String,
    pub preview_omitted: bool,
    pub preview_detail_argv: Option<Vec<String>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Ordinary,
    Info,
    Warn,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageDetails {
    pub summary: MessageSummary,
    pub content: MessageContent,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryInspection {
    pub message: MessageSummary,
    pub delivery: DeliveryAggregates,
    pub recipients: Page<Recipient>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryAggregates {
    pub committed: u64,
    pub attempted: Option<u64>,
    pub submitted: Option<u64>,
    pub read: Option<u64>,
    pub acknowledged: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageContent {
    Ordinary {
        body_data: String,
        body_offset: u64,
        body_total_bytes: u64,
        body_complete: bool,
        body_next_cursor: Option<String>,
        body_next_argv: Option<Vec<String>>,
    },
    System {
        event: StructuredEvent,
        current_condition: Option<ConditionStatus>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredEvent {
    pub kind: MessageKind,
    pub event_json: serde_json::Value,
    pub source_message: Option<MessageId>,
    pub source_invitation: Option<InvitationId>,
    pub decision_at: UtcMillis,
    pub classified_at: Option<UtcMillis>,
    pub materialized_at: Option<UtcMillis>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionStatus {
    pub active: bool,
    pub state: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    pub seat: SeatId,
    /// This row is the querying caller's own seat (the query named it as
    /// `caller`); absent otherwise.
    #[serde(rename = "self", default, skip_serializing_if = "is_false")]
    pub is_self: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<RequiredMembership>,
    pub episode: u64,
    pub joined: bool,
    pub retired: bool,
    pub physical_state: MembershipStatus,
    pub effective_state: MembershipStatus,
    pub joined_at: Option<UtcMillis>,
    pub left_at: Option<UtcMillis>,
    pub retirement_cutover: Option<UtcMillis>,
    pub cleanup_state: Option<CleanupState>,
    pub accepted_invitation: Option<InvitationAcceptance>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipStatus {
    Invited,
    Joined,
    Left,
    Retired,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvitationAcceptance {
    pub invitation: InvitationId,
    pub actor: SeatId,
    pub generation: u64,
    pub native_observation: String,
    pub accepted_at: UtcMillis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipient {
    pub seat: SeatId,
    pub status: ReceiptStatus,
    pub physical_status: ReceiptStatus,
    pub effective_status: ReceiptStatus,
    pub retirement_cutover: Option<UtcMillis>,
    pub cleanup_state: Option<CleanupState>,
    pub ack_provenance: Option<AckProvenance>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarningRecipient {
    pub seat: SeatId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AckProvenance {
    pub actor: SeatId,
    pub generation: u64,
    pub native_observation: String,
    pub decided_at: UtcMillis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    Pending,
    Acknowledged,
    Retired,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingReceipt {
    pub message: MessageId,
    pub thread: ThreadId,
    pub seat: SeatId,
    pub sequence: u64,
    pub sender: SeatId,
    pub decision_at: UtcMillis,
    pub available_at: Option<UtcMillis>,
    pub deadline: Option<UtcMillis>,
    pub overdue: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalIntent {
    pub operation: OperationId,
    pub recovery_ref: LocalRecoveryRef,
    pub created_at: UtcMillis,
    pub kind: IntentKind,
    pub thread: Option<ThreadId>,
    pub status: IntentStatus,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    ResolveSeat,
    CheckIn,
    CreateThread,
    Invite,
    Accept,
    SendMessage,
    Ack,
    Leave,
    SetTopic,
    Archive,
    Reopen,
    OperatorRebind,
    OperatorFreshSeat,
    OperatorOrphanInvite,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    Pending,
    Submitted,
    UnknownOutcome,
    KnownRejected,
    CommittedAwaitingOutput,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchPage {
    pub matches: Page<SearchHit>,
    pub examined_candidates: u16,
    pub examined_utf8_bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum SearchHit {
    Topic(ThreadSummary),
    Body(MessageSummary),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub subject: String,
    pub detail_data: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationStatus {
    pub operation: OperationId,
    pub committed: bool,
    pub result_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetirementStatus {
    pub job: RetirementJobId,
    pub seat: SeatId,
    pub effective_retired: bool,
    pub retired_at: UtcMillis,
    pub cleanup_state: CleanupState,
    pub warning_history_complete: bool,
    pub phase: String,
    pub processed_units: u64,
    pub remaining_estimate: Option<u64>,
    pub last_error: Option<BoundedError>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupState {
    Pending,
    Running,
    Complete,
    Failed,
}
/// Response metadata for installing recovered context, independent of availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckInContextDisposition {
    Current,
    Historical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckInResult {
    pub context_disposition: CheckInContextDisposition,
    /// Complete resulting context; replay returns this original context unchanged.
    pub context: crate::protocol::authority::CallerClaim,
    pub seat: SeatId,
    pub offered_through: Option<String>,
    /// Pending (actionable) warnings addressed to the seat at this check-in
    /// decision, capped at `MAX_PENDING_WARNING_COUNT`: warnings whose subject
    /// obligation (invitation or addressed receipt) is still pending, whose
    /// unavailability episode is still open, or that are programmatic service
    /// warnings not yet carried by an offer to the seat's current occupant
    /// (this offer's `notices` included). Not a historical total; a
    /// bounded-read failure rolls back.
    pub warning_count: u64,
    /// True when more than `MAX_PENDING_WARNING_COUNT` warnings are pending
    /// (`warning_count` then equals the cap).
    #[serde(default)]
    pub warning_count_has_more: bool,
    /// First page of the seat's full warning history (settled or not), with a
    /// continuation into the same canonical, read-only seatwide `warnings` route.
    pub warnings: Page<WarningRef>,
    /// The capped page of programmatic notices this offer carries and settles
    /// (at most `MAX_NOTICE_PAGE_ITEMS`); omitted on the wire when empty.
    #[serde(default, skip_serializing_if = "NoticeOffer::is_empty")]
    pub notices: NoticeOffer,
    pub inbox: Page<InboxItem>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AckResult {
    pub acknowledged: Vec<MessageId>,
    pub already_acknowledged: Vec<MessageId>,
}

/// Fixed plugin instructions are selected by code, never parsed from topic/body data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedHookInstruction {
    CheckInbox,
    RetryRegistration,
    ReadPendingReceipts,
}

/// Labels, topics and previews from peers stay visibly marked as data in hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkedPeerData {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookFragment {
    pub instruction: TrustedHookInstruction,
    pub peer_data: Vec<MarkedPeerData>,
}

pub const MAX_LAST_ERROR_BYTES: usize = 512;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BoundedError(String);
impl BoundedError {
    pub fn parse(text: impl Into<String>) -> Result<Self, &'static str> {
        let text = text.into();
        if text.len() > MAX_LAST_ERROR_BYTES {
            return Err("last error too long");
        }
        Ok(Self(text))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for BoundedError {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
