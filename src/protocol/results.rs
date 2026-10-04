use crate::protocol::service::{EventAuthor, RequiredMembership};
use crate::protocol::{ids::*, pagination::Page, time::UtcMillis};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CommandResult {
    Health(Health),
    Capabilities(CapabilityList),
    /// The daemon counted and logged a hook parse-failure report.
    HookParseFailureRecorded,
    /// The daemon recorded a hook's harness evidence note (ht-xoc.4).
    /// `verified` is the evidence row's state after recording; `false` when
    /// the note carried no version.
    HarnessEvidenceRecorded {
        verified: bool,
    },
    /// Each harness's version verdicts as the daemon derived them (ht-xoc.5).
    HarnessStates(HarnessStatesReport),
    StopAccepted(StopAccepted),
    ServiceInspection(ServiceConnectionInspection),
    ServiceDisconnected(ServiceDisconnectResult),
    Directory(Page<ThreadSummary>),
    Seats(Page<SeatSummary>),
    SeatInspect(SeatInspection),
    Inbox(Page<InboxItem>),
    InboxBatch(Page<InboxBatchItem>),
    Warnings(Page<WarningRef>),
    ActiveWarnings(Page<WarningRef>),
    Thread(ThreadDetails),
    History(Page<MessageSummary>),
    Participants(Page<Participant>),
    Recipients(Page<Recipient>),
    /// Warning recipients have no receipt or ACK obligation.
    WarningRecipients(Page<WarningRecipient>),
    DeliveryInspect(DeliveryInspection),
    PendingReceipts(Page<PendingReceipt>),
    AttentionDigest(crate::protocol::attention::AttentionDigest),
    /// Hot threads for the context-recovery hook text (spec §9).
    HotThreads(HotThreads),
    Summary(crate::protocol::summary::SummaryOutcome),
    SummaryJob(crate::protocol::summary::SummaryJobOutcome),
    SummarySubmitted(crate::protocol::summary::SubmitOutcome),
    LocalIntents(Page<LocalIntent>),
    Search(SearchPage),
    Message(MessageDetails),
    Diagnostics(Page<Diagnostic>),
    OperationStatus(OperationStatus),
    RetirementJobs(Page<RetirementStatus>),
    SeatResolved(SeatId),
    ContinuityReattached(ContinuityReattachment),
    /// The seat's open binding after a `RecordManagedLaunch` decision.
    ManagedLaunchRecorded(ManagedLaunchRecord),
    CheckedIn(CheckInResult),
    /// Local presentation of an immutable completed CheckIn fragment.
    #[serde(skip_deserializing)]
    CachedCheckInPage(crate::harness::cache::CachedCheckInPage),
    ThreadCreated(ThreadId),
    Invitation(InvitationId),
    AlreadyJoined(AlreadyJoined),
    Accepted(AcceptedInvitation),
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
    OperatorRetired(SeatId),
}

/// The result of a plain `accept`: the invitation, plus the thread when it
/// already holds one full summary chunk (spec §9 join hint). The hint is
/// computed after the commit and never stored.
///
/// Wire form: the bare invitation-id string when there is no hint (byte
/// identical to the earlier `Accepted(InvitationId)`, so stored operation
/// results and journals stay readable), else
/// `{"invitation":..,"summary_available":..}`. Both forms deserialize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedInvitation {
    pub invitation: InvitationId,
    pub summary_available: Option<ThreadId>,
}
impl From<InvitationId> for AcceptedInvitation {
    fn from(invitation: InvitationId) -> Self {
        Self {
            invitation,
            summary_available: None,
        }
    }
}
impl Serialize for AcceptedInvitation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match &self.summary_available {
            None => self.invitation.serialize(serializer),
            Some(thread) => {
                let mut out = serializer.serialize_struct("AcceptedInvitation", 2)?;
                out.serialize_field("invitation", &self.invitation)?;
                out.serialize_field("summary_available", thread)?;
                out.end()
            }
        }
    }
}
impl<'de> Deserialize<'de> for AcceptedInvitation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Bare(InvitationId),
            Full(Full),
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Full {
            invitation: InvitationId,
            #[serde(default)]
            summary_available: Option<ThreadId>,
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Bare(invitation) => Self::from(invitation),
            Wire::Full(full) => Self {
                invitation: full.invitation,
                summary_available: full.summary_available,
            },
        })
    }
}

/// Most hot threads one recovery query returns, and most overflow ids.
pub const MAX_HOT_THREADS: u32 = 8;
pub const MAX_HOT_OVERFLOW: usize = 32;
/// Longest hot-thread topic, in bytes (control characters stripped).
pub const HOT_TOPIC_BYTES: usize = 80;

/// Why a thread is hot, in ordering priority (spec §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotReason {
    PendingReceipt,
    Attention,
    Recent,
}
impl HotReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PendingReceipt => "pending_receipt",
            Self::Attention => "attention",
            Self::Recent => "recent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotThread {
    pub thread: ThreadId,
    /// Peer-controlled: control characters stripped, at most `HOT_TOPIC_BYTES`.
    pub topic_data: String,
    pub reason: HotReason,
    /// Earliest effective deadline of the seat's pending receipts in the thread.
    pub effective_deadline: Option<UtcMillis>,
    pub last_activity: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotThreads {
    pub hot: Vec<HotThread>,
    /// Hot threads past the limit (at most `MAX_HOT_OVERFLOW`), same order.
    pub overflow: Vec<ThreadId>,
}
impl HotThreads {
    pub fn validate(&self, limit: u32) -> Result<(), &'static str> {
        if self.hot.len() > limit.min(MAX_HOT_THREADS) as usize
            || self.overflow.len() > MAX_HOT_OVERFLOW
            || self
                .hot
                .iter()
                .any(|h| h.topic_data.len() > HOT_TOPIC_BYTES)
        {
            return Err("invalid hot thread bound");
        }
        Ok(())
    }
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
    /// B3 class override; never on the wire (a decoded error carries its code's default).
    #[serde(skip)]
    class_override: Option<ErrorClass>,
}

/// The most version rows a [`HarnessStateReport`] carries.
pub const HARNESS_STATE_VERSIONS: usize = 20;

/// The `harness.states` answer: one block per harness (`claude`, `codex`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessStatesReport {
    pub harnesses: Vec<HarnessStateReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessStateReport {
    pub harness: String,
    /// The contract id this harness's hooks send now (most recently seen).
    pub contract_id: Option<String>,
    /// The version on the daemon's `PATH`, with its verdict.
    pub detected: Option<DetectedVersion>,
    /// Every row under that contract id, newest `last_seen_at` first,
    /// at most [`HARNESS_STATE_VERSIONS`].
    pub versions: Vec<VersionStateReport>,
    /// The latest reason a payload could not be attributed to a version.
    pub unattributed: Option<UnattributedReport>,
    pub hook_parse_failures: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectedVersion {
    pub version: String,
    /// `working`, `new` or `broken`.
    pub state: String,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnattributedReport {
    pub reason: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionStateReport {
    pub version: String,
    /// `working`, `new` or `broken`.
    pub state: String,
    pub source: String,
    pub line: String,
    pub notes: Vec<String>,
    pub issue_url: Option<String>,
    pub last_seen_at: u64,
    pub in_health_window: bool,
}

/// The wire form of the daemon's advertised capability names (ht-p03.43).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityList {
    #[serde(default)]
    pub capabilities: Vec<String>,
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
    #[serde(default = "default_wake_batch_delay_ms")]
    pub wake_batch_delay_ms: u64,
}

// Keep the established fallback for responses that omit this field.
// Current daemons always send the resolved value, including zero.
fn default_wake_batch_delay_ms() -> u64 {
    30_000
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
            settings.wake_batch_delay_ms > i64::MAX as u64
                || [
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
/// A seat's open (not ended) binding, the single answer every A4 guard reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenBindingSummary {
    pub provenance: String,
    pub harness: String,
    pub target: HostTargetId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatInspection {
    pub summary: SeatSummary,
    pub mapping: MappingStatus,
    pub hold: Option<HoldSummary>,
    pub retirement: Option<RetirementStatus>,
    /// The seat's open binding, independent of the history page.
    #[serde(default)]
    pub open_binding: Option<OpenBindingSummary>,
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
    /// TRUST-POLICY C1: how Herdr's `agent_session` compared with the resumed
    /// session id (`match`, `mismatch`, `absent`, `read_error`). Diagnostic
    /// only; it never decided the reattachment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuity_diagnostic: Option<String>,
}
/// The outcome of one `RecordManagedLaunch` (TRUST-POLICY A3
/// `managed_launch`): `recorded` when this decision opened the seat's
/// unregistered `managed_launch` binding, otherwise the seat already had an
/// open binding, left unchanged and described here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedLaunchRecord {
    pub seat: SeatId,
    pub recorded: bool,
    pub binding_generation: u64,
    pub provenance: String,
}
/// The seat a resumed session was reattached to and the generation of the
/// successor binding the same transaction opened; the caller writes its
/// context from it and sends nothing further.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuityReattachment {
    pub seat: SeatId,
    pub binding_generation: u64,
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

/// One complete copyable inbox action, or one UTF-8 aligned body chunk.
/// `ack_candidate` is present only for a fully displayed canonical pending
/// agent receipt; consumers commit it only after writing and flushing output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InboxBatchItem {
    Invitation {
        thread: ThreadId,
        topic_data: String,
        invitation: InvitationId,
        required_service: Option<RequiredMembership>,
    },
    Message {
        thread: ThreadId,
        topic_data: String,
        message: MessageId,
        sequence: u64,
        sender: Option<SeatId>,
        body: String,
        body_start: u64,
        body_end: u64,
        body_len: u64,
        ack_candidate: Option<MessageId>,
    },
    Warning {
        thread: ThreadId,
        topic_data: String,
        warning: MessageId,
        sequence: u64,
    },
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
    /// Spec §1: recorded (or, for pre-migration rows, backfilled) author role. Absent = NULL, read as agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_role: Option<crate::protocol::summary::AuthorRole>,
    /// The sender's cooperative `send --relays-user` claim.
    #[serde(default, skip_serializing_if = "is_false")]
    pub relays_user: bool,
    /// The role was derived at migration, not recorded at send.
    #[serde(default, skip_serializing_if = "is_false")]
    pub author_role_backfilled: bool,
    pub kind: MessageKind,
    pub sequence: u64,
    pub created_at: UtcMillis,
    pub actor_label: Option<String>,
    pub preview_data: String,
    pub preview_omitted: bool,
    pub preview_detail_argv: Option<Vec<String>>,
}
impl MessageSummary {
    /// Fixed-text authorship markers for renderers: `[human]` and/or
    /// `[relays user]`, each preceded by one space; empty when neither applies.
    /// Never derived from peer data.
    pub fn author_markers(&self) -> String {
        let mut markers = String::new();
        if self.author_role == Some(crate::protocol::summary::AuthorRole::Human) {
            markers.push_str(" [human]");
        }
        if self.relays_user {
            markers.push_str(" [relays user]");
        }
        markers
    }
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
    /// Frozen deadline of a pending receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<UtcMillis>,
    /// Later deadline while a catch-up extension is in force (spec §8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_deadline: Option<UtcMillis>,
    /// The effective deadline while it is still ahead of now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_until: Option<UtcMillis>,
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
    NotRequired,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingReceipt {
    pub message: MessageId,
    pub thread: ThreadId,
    pub seat: SeatId,
    pub sequence: u64,
    /// The native sender seat; `None` for a service-authored message.
    pub sender: Option<SeatId>,
    /// Set only when the sender is not a native seat (a programmatic service
    /// author), so native rows serialize exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_author: Option<crate::protocol::service::EventAuthor>,
    pub decision_at: UtcMillis,
    pub available_at: Option<UtcMillis>,
    pub deadline: Option<UtcMillis>,
    pub overdue: bool,
    /// Later deadline while a catch-up extension is in force (spec §8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_deadline: Option<UtcMillis>,
    /// The effective deadline while it is still ahead of now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_until: Option<UtcMillis>,
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
    OperatorRetire,
    OperatorReplace,
    ContinuityCheckIn,
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

/// B3 failure taxonomy (root spec §B3 Decision 3). Derived from the code
/// until ht-p03.22 lands a stored, overridable field; never on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorClass {
    Transient,
    Unavailable,
    Corrupt,
    VersionSkew,
}

impl ErrorCode {
    /// Every code, in declaration order.
    pub const ALL: &'static [ErrorCode] = &[
        ErrorCode::Unsupported,
        ErrorCode::InvalidRequest,
        ErrorCode::UnknownWireVersion,
        ErrorCode::DaemonVersionMismatch,
        ErrorCode::InstanceMismatch,
        ErrorCode::DaemonBootChanged,
        ErrorCode::CursorStale,
        ErrorCode::InvalidCursor,
        ErrorCode::InvalidBudget,
        ErrorCode::ReadBudgetExhausted,
        ErrorCode::SequenceExhausted,
        ErrorCode::Unauthorized,
        ErrorCode::CallerUnverified,
        ErrorCode::PermitExpired,
        ErrorCode::StaleHostObservation,
        ErrorCode::TargetUnresolved,
        ErrorCode::TargetUnsafe,
        ErrorCode::TargetAlreadyOwned,
        ErrorCode::ThreadNotOrphaned,
        ErrorCode::Archived,
        ErrorCode::NotFound,
        ErrorCode::OperationPayloadMismatch,
        ErrorCode::StoreBusy,
        ErrorCode::StoreCorrupt,
        ErrorCode::StoreFull,
        ErrorCode::IncompatibleSchema,
        ErrorCode::HostUnavailable,
        ErrorCode::UnknownOutcome,
        ErrorCode::MissingHook,
        ErrorCode::UnsupportedHarness,
        ErrorCode::Cancelled,
        ErrorCode::DeadlineExceeded,
        ErrorCode::Conflict,
        ErrorCode::ServiceBusy,
        ErrorCode::ServiceNotRegistered,
        ErrorCode::StaleServiceGeneration,
        ErrorCode::IncompatibleOwnership,
        ErrorCode::RequiredInvitationNeedsManagedThread,
        ErrorCode::MembershipRequired,
        ErrorCode::StaleRequirementAcceptance,
        ErrorCode::TransportDenied,
    ];

    /// The failure class this code defaults to, or `None` for codes that
    /// describe a caller/request problem rather than a failure. Exhaustive: a
    /// new code cannot compile without a class decision.
    pub fn default_class(&self) -> Option<ErrorClass> {
        match self {
            ErrorCode::Unsupported => None,
            ErrorCode::InvalidRequest => None,
            ErrorCode::UnknownWireVersion => Some(ErrorClass::VersionSkew),
            ErrorCode::DaemonVersionMismatch => Some(ErrorClass::VersionSkew),
            ErrorCode::InstanceMismatch => Some(ErrorClass::Transient),
            ErrorCode::CursorStale => Some(ErrorClass::Transient),
            ErrorCode::InvalidCursor => None,
            ErrorCode::InvalidBudget => None,
            ErrorCode::ReadBudgetExhausted => Some(ErrorClass::Transient),
            ErrorCode::SequenceExhausted => None,
            ErrorCode::Unauthorized => None,
            ErrorCode::CallerUnverified => None,
            ErrorCode::PermitExpired => Some(ErrorClass::Transient),
            ErrorCode::StaleHostObservation => Some(ErrorClass::Transient),
            ErrorCode::TargetUnresolved => None,
            ErrorCode::TargetUnsafe => None,
            ErrorCode::TargetAlreadyOwned => None,
            ErrorCode::ThreadNotOrphaned => None,
            ErrorCode::Archived => None,
            ErrorCode::NotFound => None,
            ErrorCode::OperationPayloadMismatch => None,
            ErrorCode::StoreBusy => Some(ErrorClass::Transient),
            ErrorCode::StoreCorrupt => Some(ErrorClass::Corrupt),
            ErrorCode::StoreFull => Some(ErrorClass::Transient),
            ErrorCode::IncompatibleSchema => Some(ErrorClass::VersionSkew),
            ErrorCode::HostUnavailable => Some(ErrorClass::Unavailable),
            ErrorCode::UnknownOutcome => Some(ErrorClass::Transient),
            ErrorCode::MissingHook => None,
            ErrorCode::UnsupportedHarness => None,
            ErrorCode::Cancelled => None,
            ErrorCode::DeadlineExceeded => Some(ErrorClass::Transient),
            ErrorCode::Conflict => None,
            ErrorCode::ServiceBusy => Some(ErrorClass::Transient),
            ErrorCode::ServiceNotRegistered => Some(ErrorClass::Unavailable),
            ErrorCode::StaleServiceGeneration => Some(ErrorClass::Transient),
            ErrorCode::IncompatibleOwnership => Some(ErrorClass::VersionSkew),
            ErrorCode::RequiredInvitationNeedsManagedThread => None,
            ErrorCode::MembershipRequired => None,
            ErrorCode::StaleRequirementAcceptance => None,
            ErrorCode::TransportDenied => Some(ErrorClass::Unavailable),
            // The daemon restarted between the caller's view and the request
            // (B5, protocol 2): retry against the new boot.
            ErrorCode::DaemonBootChanged => Some(ErrorClass::Transient),
        }
    }
}

impl ApiError {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            restart_argv: None,
            required_minimum_bytes: None,
            class_override: None,
        }
    }

    pub fn with_restart_argv(mut self, argv: Vec<String>) -> Self {
        self.restart_argv = Some(argv);
        self
    }

    pub fn with_required_minimum_bytes(mut self, bytes: u32) -> Self {
        self.required_minimum_bytes = Some(bytes);
        self
    }

    /// The failure class: the override when set, else the code's default.
    pub fn class(&self) -> Option<ErrorClass> {
        self.class_override.or_else(|| self.code.default_class())
    }

    pub fn with_class(mut self, class: ErrorClass) -> Self {
        self.class_override = Some(class);
        self
    }
}

macro_rules! api_error_constructors {
    ($($name:ident => $code:ident),* $(,)?) => {
        impl ApiError {
            $(
                #[doc = concat!("An `ErrorCode::", stringify!($code), "` error with no restart argv.")]
                pub fn $name(detail: impl Into<String>) -> Self { Self::new(ErrorCode::$code, detail) }
            )*
            /// The constructor for `code` (exhaustive: every code has one).
            pub fn constructor_for(code: ErrorCode) -> fn(&str) -> ApiError {
                match code { $(ErrorCode::$code => |d: &str| ApiError::$name(d),)* }
            }
        }
    };
}
api_error_constructors! {
    unsupported => Unsupported, invalid_request => InvalidRequest,
    unknown_wire_version => UnknownWireVersion, daemon_version_mismatch => DaemonVersionMismatch,
    instance_mismatch => InstanceMismatch, cursor_stale => CursorStale, invalid_cursor => InvalidCursor,
    invalid_budget => InvalidBudget, read_budget_exhausted => ReadBudgetExhausted,
    sequence_exhausted => SequenceExhausted, unauthorized => Unauthorized,
    caller_unverified => CallerUnverified, permit_expired => PermitExpired,
    stale_host_observation => StaleHostObservation, target_unresolved => TargetUnresolved,
    target_unsafe => TargetUnsafe, target_already_owned => TargetAlreadyOwned,
    thread_not_orphaned => ThreadNotOrphaned, archived => Archived, not_found => NotFound,
    operation_payload_mismatch => OperationPayloadMismatch, store_busy => StoreBusy,
    store_corrupt => StoreCorrupt, store_full => StoreFull, incompatible_schema => IncompatibleSchema,
    host_unavailable => HostUnavailable, unknown_outcome => UnknownOutcome, missing_hook => MissingHook,
    unsupported_harness => UnsupportedHarness, cancelled => Cancelled,
    deadline_exceeded => DeadlineExceeded, conflict => Conflict, service_busy => ServiceBusy,
    service_not_registered => ServiceNotRegistered, stale_service_generation => StaleServiceGeneration,
    incompatible_ownership => IncompatibleOwnership,
    required_invitation_needs_managed_thread => RequiredInvitationNeedsManagedThread,
    membership_required => MembershipRequired, stale_requirement_acceptance => StaleRequirementAcceptance,
    transport_denied => TransportDenied, daemon_boot_changed => DaemonBootChanged,
}

#[cfg(test)]
#[path = "../../tests/protocol/error_class.rs"]
mod error_class_tests;
