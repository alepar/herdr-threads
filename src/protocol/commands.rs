use crate::protocol::{
    authority::CallerClaim,
    ids::*,
    pagination::{Cursor, CursorScope, MAX_PAGE_BYTES, PageRequest},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "args", rename_all = "snake_case")]
pub enum Command {
    Health,
    /// Daemon capability discovery (ht-p03.43); a separate request because the
    /// Health hello is `deny_unknown_fields` in shipped CLIs.
    Capabilities,
    /// A hook reports a payload its optimistically admitted recipe could not
    /// parse (ht-p03.23); sent only to a daemon advertising
    /// `hook.parse_failure_report`.
    HookParseFailure(HookParseFailure),
    /// A hook reports what a payload showed about the harness that sent it
    /// (ht-xoc.4); sent only to a daemon advertising `hook.harness_evidence`.
    HarnessEvidence(HarnessEvidence),
    /// Doctor asks for each harness's version verdicts (ht-xoc.5); sent only
    /// to a daemon advertising `harness.states`.
    HarnessStates,
    Stop(StopRequest),
    ServiceInspect,
    ServiceDisconnect(ServiceDisconnectRequest),
    Directory(DirectoryQuery),
    /// Bounded human picker metrics, gated by picker.directory_v1.
    PickerDirectory(PickerDirectoryQuery),
    Seats(SeatsQuery),
    SeatInspect(SeatInspectQuery),
    Inbox(InboxQuery),
    /// Bounded read-only content page for the text inbox. Receipt changes use
    /// a separate accountable command after the caller displays this page.
    InboxBatch(InboxQuery),
    Warnings(WarningsQuery),
    /// Bounded read-only active warning conditions for one thread.
    ActiveWarnings(ActiveWarningsQuery),
    Thread(ThreadQuery),
    /// Indexed exact ID/name lookup across the selected instance.
    ResolveThread(ResolveThreadQuery),
    ThreadName(ThreadNameQuery),
    History(HistoryQuery),
    Participants(ParticipantsQuery),
    ParticipantLocations(ParticipantLocationsQuery),
    Recipients(RecipientsQuery),
    DeliveryInspect(DeliveryInspectQuery),
    PendingReceipts(PendingReceiptsQuery),
    /// Read-only seat attention digest; never mutates receipts, ACK or checkpoints.
    AttentionDigest(AttentionDigestQuery),
    /// Read-only v1 digest plus exact-occupant informational delivery readiness.
    /// Separate envelope preserves shipped digest decoders and logical tokens.
    AttentionDigestDelivery(AttentionDigestQuery),
    /// Read-only hot threads of a seat for the recovery hook text (spec §9).
    HotThreads(HotThreadsQuery),
    /// Thread summary protocol (spec §4). Each carries the seat's claim; a
    /// declared subagent (summary worker) may issue all three.
    Summary(crate::protocol::summary::SummaryRequest),
    SummaryJob(crate::protocol::summary::SummaryJobRequest),
    SummarySubmit(crate::protocol::summary::SummarySubmitRequest),
    LocalIntents(LocalIntentsQuery),
    Search(SearchQuery),
    Message(MessageQuery),
    Diagnostics(DiagnosticsQuery),
    OperationStatus(OperationStatusQuery),
    RetirementJobs(RetirementJobsQuery),
    ResolveSeat(ResolveSeat),
    /// TRUST-POLICY C1: seatless resume-only reattachment decision.
    ContinuityCheckIn(ContinuityCheckIn),
    /// TRUST-POLICY A3 `managed_launch`: `launch` reports a correlated,
    /// observed startup so the daemon can open an unregistered occupant
    /// binding on a seat with no open binding (ht-5n6). Sent only to a daemon
    /// advertising `seat.managed_launch`.
    RecordManagedLaunch(RecordManagedLaunch),
    CheckIn(CheckIn),
    /// Person check-in over an agent's binding, as the local account
    /// (TRUST-POLICY A4 override). Human lifecycle only.
    OperatorCheckIn(CheckIn),
    BeginHandoff(crate::protocol::handoff::HandoffMutation),
    CompleteHandoff(crate::protocol::handoff::HandoffMutation),
    CreateThread(CreateThread),
    Invite(Invite),
    Accept(Accept),
    AcceptRequired(AcceptRequired),
    Reject(Reject),
    SendMessage(SendMessage),
    Ack(Ack),
    /// Accountable ACK claimed only after a whole inbox text page is flushed.
    AckDisplayed(Ack),
    Leave(Leave),
    SetTopic(SetTopic),
    SetThreadName(SetThreadName),
    Archive(ThreadMutation),
    Reopen(ThreadMutation),
    OperatorRebind(OperatorRebind),
    OperatorFreshSeat(OperatorFreshSeat),
    OperatorOrphanInvite(OperatorOrphanInvite),
    OperatorRetire(OperatorRetire),
    OperatorReplace(OperatorReplace),
    BeginBootstrap(Box<crate::protocol::handoff::BeginBootstrap>),
    ReserveBootstrapAttempt(Box<crate::protocol::handoff::ReserveBootstrapAttempt>),
    RecordBootstrapCreated(Box<crate::protocol::handoff::RecordBootstrapCreated>),
    RecordBootstrapNotSubmitted(Box<crate::protocol::handoff::RecordBootstrapNotSubmitted>),
    AttachBootstrapHandoff(Box<crate::protocol::handoff::AttachBootstrapHandoff>),
    CompleteLinkedBootstrap(Box<crate::protocol::handoff::CompleteLinkedBootstrap>),
    CheckBootstrapSubmission(Box<crate::protocol::handoff::CheckBootstrapSubmission>),
    BootstrapStatus(Box<crate::protocol::handoff::BootstrapStatus>),
    ResolveBootstrapSeat(Box<crate::protocol::handoff::ResolveBootstrapSeat>),
    RecoverBootstrap(Box<crate::protocol::handoff::RecoverBootstrap>),
}

/// Longest detail a hook report may carry: the CLI truncates to it
/// ([`bounded_hook_detail`]) and validation rejects a longer one.
pub const HOOK_PARSE_DETAIL_BYTES: usize = 256;

/// `text` cut to at most [`HOOK_PARSE_DETAIL_BYTES`] bytes at a character
/// boundary, so the report always passes the byte bound (a character bound
/// would let multi-byte text through at up to four times the limit).
pub fn bounded_hook_detail(text: &str) -> String {
    let mut end = text.len().min(HOOK_PARSE_DETAIL_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// One hook payload that did not parse under an optimistic admission. The
/// detail is the hook's own bounded diagnostic, never the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookParseFailure {
    pub harness: String,
    pub detail: String,
}

/// Longest `field` and `unattributed_reason` a [`HarnessEvidence`] may carry.
pub const HARNESS_EVIDENCE_TEXT_BYTES: usize = 128;
/// Longest `session_id` a [`HarnessEvidence`] may carry.
pub const HARNESS_EVIDENCE_SESSION_BYTES: usize = 256;

/// What one hook payload said about its harness version's contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HarnessEvidenceOutcome {
    Ok,
    Violation { field: String },
    Malformed,
}

/// One hook's evidence note (ht-xoc.4): no payload content, only the harness,
/// its attributed version (or why there is none), the hook's own contract id,
/// the event, the outcome and the session id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessEvidence {
    /// `claude` or `codex`.
    pub harness: String,
    /// Canonical `X.Y.Z`; `None` when the payload could not be attributed.
    pub version: Option<String>,
    /// Why there is no version; present exactly when `version` is `None`.
    pub unattributed_reason: Option<String>,
    /// The hook's contract id: 16 lowercase hex.
    pub contract_id: String,
    /// The registered (or discriminator) event name: 1..=63 ASCII alphanumerics.
    pub event: String,
    pub outcome: HarnessEvidenceOutcome,
    pub session_id: Option<String>,
}

impl HarnessEvidence {
    pub fn validate(&self) -> Result<(), &'static str> {
        const BAD: &str = "invalid harness evidence";
        if !matches!(self.harness.as_str(), "claude" | "codex") {
            return Err(BAD);
        }
        match (&self.version, &self.unattributed_reason) {
            (Some(version), None) => {
                if crate::harness::contract::normalize_version(&self.harness, version).as_deref()
                    != Some(version.as_str())
                {
                    return Err(BAD);
                }
            }
            (None, Some(reason)) => {
                if reason.is_empty() || reason.len() > HARNESS_EVIDENCE_TEXT_BYTES {
                    return Err(BAD);
                }
            }
            _ => return Err(BAD),
        }
        if self.contract_id.len() != 16
            || !self
                .contract_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(BAD);
        }
        if self.event.is_empty()
            || self.event.len() > 63
            || !self.event.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(BAD);
        }
        if let HarnessEvidenceOutcome::Violation { field } = &self.outcome
            && (field.is_empty() || field.len() > HARNESS_EVIDENCE_TEXT_BYTES)
        {
            return Err(BAD);
        }
        if self
            .session_id
            .as_ref()
            .is_some_and(|id| id.len() > HARNESS_EVIDENCE_SESSION_BYTES)
        {
            return Err(BAD);
        }
        Ok(())
    }
}

/// Service control only. The envelope supplies the expected instance; the
/// daemon checks this boot before accepting drain and owner release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopRequest {
    pub expected_boot: String,
}

/// The operator must name the exact observed connection. This carries no
/// service authority and is accepted only through the local same-UID path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDisconnectRequest {
    pub expected_boot: String,
    pub expected_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickerDirectoryQuery {
    pub page: PageRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryQuery {
    #[serde(default, skip_serializing_if = "is_false")]
    pub recent: bool,
    pub membership: Option<SeatId>,
    pub membership_filter: DirectoryMembership,
    pub topic_contains: Option<String>,
    pub page: PageRequest,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryMembership {
    #[default]
    Default,
    Joined,
    Invited,
    All,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatsQuery {
    pub page: PageRequest,
    /// Optional pane filter: only seat(s) mapped to this target. Includes
    /// retired seats only when `include_retired` is true. Absent when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<HostTargetId>,
    /// Include retired seats in the directory. Absent means active only.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_retired: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatInspectQuery {
    pub seat: SeatId,
    pub page: PageRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboxQuery {
    pub seat: Option<SeatId>,
    pub page: PageRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarningsQuery {
    pub seat: SeatId,
    pub page: PageRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveWarningsQuery {
    pub thread: ThreadId,
    pub page: PageRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadQuery {
    pub thread: ThreadId,
    pub page: PageRequest,
    /// The CLI caller's own seat, when the invoking pane maps to one: the
    /// participant row for it is marked `"self": true` (native codex matrix
    /// P2). Presentation only; it scopes and authorizes nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<SeatId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    pub thread: ThreadId,
    pub page: PageRequest,
    pub initial: Option<HistoryRange>,
    /// Inline each ordinary message's complete body in its summary when the
    /// body is small enough that a `Message` fetch would return it whole
    /// (`FULL_BODY_FETCH_BYTES`) and the page budget holds it. Sent only to a
    /// daemon that advertises `capabilities::HISTORY_FULL_BODIES`; omitted from
    /// the wire when false so existing request bytes are unchanged.
    #[serde(default, skip_serializing_if = "is_false")]
    pub full_bodies: bool,
}
/// Output byte bound of the `Message` fetch the human `read` issues for a
/// clipped preview; also the measure of a body the daemon may inline.
pub const FULL_BODY_FETCH_BYTES: u32 = 16_384;
fn is_false(value: &bool) -> bool {
    !*value
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryRange {
    Recent { count: u16 },
    After { sequence: u64 },
    Before { sequence: u64 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantsQuery {
    pub thread: ThreadId,
    pub page: PageRequest,
    /// The CLI caller's own seat, when the invoking pane maps to one: the
    /// participant row for it is marked `"self": true` (native codex matrix
    /// P2). Presentation only; it scopes and authorizes nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<SeatId>,
}
/// Bounded canonical mappings for one already displayed participant page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantLocationsQuery {
    pub thread: ThreadId,
    pub seats: Vec<SeatId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipientsQuery {
    pub message: MessageId,
    pub page: PageRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryInspectQuery {
    pub message: MessageId,
    pub page: PageRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingReceiptsQuery {
    pub seat: Option<SeatId>,
    pub thread: Option<ThreadId>,
    pub page: PageRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalIntentsQuery {
    pub page: PageRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchQuery {
    pub literal: String,
    pub thread: Option<ThreadId>,
    pub page: PageRequest,
    pub max_candidates: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageQuery {
    pub message: MessageId,
    pub body: BodyReadRequest,
}

/// The bound includes JSON framing and both body/page continuations in the response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyReadRequest {
    pub cursor: Option<String>,
    pub offset: Option<u64>,
    pub max_bytes: u32,
}
impl BodyReadRequest {
    pub fn validate(&self, message: &MessageId) -> Result<(), &'static str> {
        if self.max_bytes < 256 || self.max_bytes > MAX_PAGE_BYTES {
            return Err("invalid body output byte bound");
        }
        if self.cursor.is_some() && self.offset.is_some() {
            return Err("body cursor conflicts with initial offset");
        }
        if let Some(encoded) = &self.cursor {
            let cursor = Cursor::decode(encoded)?;
            if cursor.scope != CursorScope::MessageBody
                || !cursor.may_name_scope_key(message.as_str())
            {
                return Err("body cursor scope mismatch");
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionDigestQuery {
    pub seat: SeatId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotThreadsQuery {
    pub seat: SeatId,
    /// Hot threads returned in full, `1..=8`; the rest come back as overflow ids.
    pub limit: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsQuery {
    pub seat: Option<SeatId>,
    pub thread: Option<ThreadId>,
    pub page: PageRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationStatusQuery {
    pub operation: OperationId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetirementJobsQuery {
    pub page: PageRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveSeat {
    pub target: HostTargetId,
    pub operation: OperationId,
}
/// TRUST-POLICY C1. A resumed top-level session asks the daemon to reattach
/// the one unresolved seat whose last binding carries its session id onto the
/// pane's target. It names no seat: the daemon decides the seat, rebinds it and
/// opens the successor `cooperative_top_level` binding for `execution` in the
/// same transaction; there is no follow-up lifecycle check-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuityCheckIn {
    pub target: HostTargetId,
    pub harness: crate::protocol::authority::Harness,
    pub native_session: NativeSessionId,
    /// The hook's SessionStart source. Only `resume` can reattach.
    pub source: String,
    pub operation: OperationId,
    /// The execution id the successor binding carries; chosen by the client
    /// once per intent so every retry under the operation key repeats it.
    pub execution: ExecutionId,
}
impl ContinuityCheckIn {
    pub const RESUME_SOURCE: &'static str = "resume";
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.source != Self::RESUME_SOURCE {
            return Err("only a resume check-in can reattach a seat");
        }
        if self.harness == crate::protocol::authority::Harness::Human {
            return Err("a human occupant cannot resume a harness session");
        }
        if self.native_session.as_str().is_empty()
            || self.native_session.as_str().starts_with("plugin_context:")
        {
            return Err("a resumed session needs a native session id");
        }
        Ok(())
    }
}
/// The startup evidence of one accepted managed launch (a host-correlated
/// `ObservedStartup`): which seat's pane Herdr started which harness in, and
/// the structural identity (terminal, Herdr incarnation and boot, target
/// generation) the launcher observed. The daemon decides against its own
/// effective observation; these values only have to agree with it. The
/// launcher's host epoch is its own connection counter and is not sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordManagedLaunch {
    pub seat: SeatId,
    pub target: HostTargetId,
    pub harness: crate::protocol::authority::Harness,
    pub terminal: TerminalId,
    pub incarnation: String,
    pub host_boot: HostBootId,
    pub target_generation: u64,
}
impl RecordManagedLaunch {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.harness == crate::protocol::authority::Harness::Human {
            return Err("a managed launch starts an agent, never a person");
        }
        if self.incarnation.is_empty()
            || self.incarnation.len() > 128
            || self.terminal.as_str().is_empty()
            || self.host_boot.as_str().is_empty()
        {
            return Err("managed launch evidence out of range");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckIn {
    pub mode: CheckInMode,
    pub claim: CallerClaim,
    pub operation: OperationId,
}
/// Lifecycle requests deliberately rotate using a fixed expected generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CheckInMode {
    Current,
    Lifecycle { expected_binding_generation: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateThread {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub topic: String,
    pub goal: String,
    pub operation: OperationId,
    pub claim: CallerClaim,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invite {
    pub thread: ThreadId,
    pub seat: SeatId,
    pub deadline_millis: Option<u64>,
    pub operation: OperationId,
    pub claim: CallerClaim,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accept {
    pub thread: ThreadId,
    pub operation: OperationId,
    pub claim: CallerClaim,
}
/// Reject only the exact ordinary invitation the recipient inspected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reject {
    pub thread: ThreadId,
    pub invitation: InvitationId,
    pub reason: String,
    pub operation: OperationId,
    pub claim: CallerClaim,
}

pub const MAX_REJECTION_REASON_BYTES: usize = 4096;
pub fn validate_rejection_reason(reason: &str) -> Result<(), &'static str> {
    if reason.trim().is_empty() || reason.len() > MAX_REJECTION_REASON_BYTES {
        return Err("rejection reason must be nonblank and at most 4096 UTF-8 bytes");
    }
    Ok(())
}

/// A native caller must name the requirement episode and revision it saw.
/// An ordinary acceptance key cannot accept a later required upgrade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptRequired {
    pub thread: ThreadId,
    pub invitation: InvitationId,
    pub requirement: RequirementId,
    pub expected_revision: u64,
    pub operation: OperationId,
    pub claim: CallerClaim,
}
impl AcceptRequired {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.expected_revision == 0 {
            return Err("required acceptance revision must be positive");
        }
        if self.claim.role == crate::protocol::authority::CallerRole::Subagent {
            return Err("declared subagents cannot accept requirements");
        }
        Ok(())
    }

    /// The currently addressed episode must still have exactly the semantics
    /// the native caller saw. Store callers perform this check before deciding
    /// a new acceptance, including after an ordinary invitation was upgraded.
    // Allowed: the error deliberately carries the current RequiredMembership so
    // callers can reread it; boxing would change the protocol error type.
    #[allow(clippy::result_large_err)]
    pub fn check_current(
        &self,
        current: &crate::protocol::service::RequiredMembership,
    ) -> Result<(), crate::protocol::service::RequiredAcceptanceError> {
        use crate::protocol::service::{RequiredAcceptanceError, RequirementState};
        if self.claim.seat != current.seat || self.thread != current.thread {
            return Err(RequiredAcceptanceError::WrongParticipant);
        }
        if self.invitation != current.invitation
            || self.requirement != current.requirement
            || self.expected_revision != current.revision
        {
            return Err(RequiredAcceptanceError::Stale(current.clone()));
        }
        if current.state != RequirementState::Pending {
            return Err(RequiredAcceptanceError::NotPending(current.clone()));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_intent: Option<crate::protocol::summary::UserIntent>,
    pub thread: ThreadId,
    pub body: String,
    pub invited_recipients: Vec<SeatId>,
    pub deadline_millis: Option<u64>,
    pub operation: OperationId,
    pub claim: CallerClaim,
    /// Spec §1: the sender claims this message relays its user's instruction
    /// (`send --relays-user`). A cooperative claim (TRUST-POLICY A1, A3); service sends record 0.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relays_user: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ack {
    pub messages: Vec<MessageId>,
    pub operation: OperationId,
    pub claim: CallerClaim,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Leave {
    pub thread: ThreadId,
    pub operation: OperationId,
    pub claim: CallerClaim,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetTopic {
    pub thread: ThreadId,
    pub topic: String,
    pub operation: OperationId,
    pub claim: CallerClaim,
}
/// Names are exact, single-line UTF-8; size is measured in bytes.
pub fn validate_thread_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty()
        || name.len() > 128
        || name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}'))
    {
        return Err("thread name must be 1..128 UTF-8 bytes with no control characters");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadNameQuery {
    pub thread: ThreadId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveThreadQuery {
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_target: Option<HostTargetId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<SeatId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetThreadName {
    pub thread: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub operation: OperationId,
    pub claim: CallerClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadMutation {
    pub thread: ThreadId,
    pub operation: OperationId,
    pub claim: CallerClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorRebind {
    pub seat: SeatId,
    pub target: HostTargetId,
    pub operation: OperationId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorFreshSeat {
    pub target: HostTargetId,
    pub operation: OperationId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorOrphanInvite {
    pub thread: ThreadId,
    pub seat: SeatId,
    pub deadline_millis: Option<u64>,
    pub operation: OperationId,
}

/// Abandon a seat on the operator's say-so (TRUST-POLICY C3). Retirement is
/// not a target claim, so no host observation accompanies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorRetire {
    pub seat: SeatId,
    pub operation: OperationId,
}
/// Retire `replace` (the seat now owning `target`) and rebind `seat` onto
/// `target` in one deciding transaction. Nothing moves from `replace` to `seat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorReplace {
    pub seat: SeatId,
    pub target: HostTargetId,
    pub replace: SeatId,
    pub operation: OperationId,
}

pub const MAX_SEARCH_CANDIDATES: u16 = 100;
pub const MAX_BATCH_ITEMS: usize = 100;

impl Command {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::BeginBootstrap(v) => {
                v.identity.validate()?;
                if v.operation != v.identity.payload.handoff.keys.begin {
                    return Err("bootstrap begin key mismatch");
                }
                return Ok(());
            }
            Self::ReserveBootstrapAttempt(v) => {
                v.identity.validate()?;
                if v.operation
                    != v.expected_attempt
                        .operation(&v.identity.compound, "reserve")?
                {
                    return Err("bootstrap reserve key mismatch");
                }
                return Ok(());
            }
            Self::RecordBootstrapNotSubmitted(v) => {
                v.identity.validate()?;
                if v.operation
                    != v.expected_attempt
                        .operation(&v.identity.compound, "not_submitted")?
                {
                    return Err("bootstrap not-submitted key mismatch");
                }
                return Ok(());
            }
            Self::RecordBootstrapCreated(v) => {
                v.identity.validate()?;
                v.evidence.validate()?;
                if v.operation
                    != v.expected_attempt
                        .operation(&v.identity.compound, "record")?
                    || v.evidence.workspace != v.identity.payload.workspace
                    || v.evidence.witness.endpoint
                        != v.identity.payload.handoff.namespace.host_endpoint
                {
                    return Err("bootstrap record mismatch");
                }
                return Ok(());
            }
            Self::AttachBootstrapHandoff(v) => {
                v.identity.validate()?;
                v.attachment.validate(&v.identity)?;
                if v.operation != v.identity.payload.attach_key {
                    return Err("bootstrap attach key mismatch");
                }
                return Ok(());
            }
            Self::CompleteLinkedBootstrap(v) => return v.validate(),
            Self::CheckBootstrapSubmission(v) => {
                v.identity.validate()?;
                if v.operation
                    != v.expected_attempt
                        .operation(&v.identity.compound, "check")?
                {
                    return Err("bootstrap check key mismatch");
                }
                return Ok(());
            }
            Self::BootstrapStatus(v) => return v.identity.validate(),
            Self::ResolveBootstrapSeat(v) => return v.validate(),
            Self::RecoverBootstrap(v) => {
                v.identity.validate()?;
                v.disposition.validate()?;
                if v.operation != v.decision_operation()? {
                    return Err("bootstrap recovery key mismatch");
                }
                return Ok(());
            }
            _ => {}
        }
        if let Some(page) = self.page() {
            page.validate()?;
        }
        let claim = match self {
            Self::CheckIn(v) | Self::OperatorCheckIn(v) => Some(&v.claim),
            Self::BeginHandoff(v) | Self::CompleteHandoff(v) => Some(&v.identity.claim),
            Self::CreateThread(v) => Some(&v.claim),
            Self::Invite(v) => Some(&v.claim),
            Self::Accept(v) => Some(&v.claim),
            Self::AcceptRequired(v) => Some(&v.claim),
            Self::Reject(v) => Some(&v.claim),
            Self::SendMessage(v) => Some(&v.claim),
            Self::Ack(v) | Self::AckDisplayed(v) => Some(&v.claim),
            Self::Leave(v) => Some(&v.claim),
            Self::SetTopic(v) => Some(&v.claim),
            Self::SetThreadName(v) => Some(&v.claim),
            Self::Archive(v) | Self::Reopen(v) => Some(&v.claim),
            _ => None,
        };
        if claim.is_some_and(|v| v.role == crate::protocol::authority::CallerRole::Subagent) {
            return Err("declared subagents cannot perform accountable operations");
        }
        match self {
            Self::Stop(stop)
                if uuid::Uuid::parse_str(&stop.expected_boot)
                    .map_or(true, |boot| boot.to_string() != stop.expected_boot) =>
            {
                Err("invalid expected daemon boot")
            }
            Self::ServiceDisconnect(disconnect)
                if disconnect.expected_generation == 0
                    || uuid::Uuid::parse_str(&disconnect.expected_boot)
                        .map_or(true, |boot| boot.to_string() != disconnect.expected_boot) =>
            {
                Err("invalid expected service boot or generation")
            }
            Self::HookParseFailure(report)
                if !matches!(report.harness.as_str(), "claude" | "codex")
                    || report.detail.len() > HOOK_PARSE_DETAIL_BYTES =>
            {
                Err("invalid hook parse-failure report")
            }
            Self::HarnessEvidence(evidence) => evidence.validate(),
            Self::History(query) if query.initial.is_some() && query.page.cursor.is_some() => {
                Err("history selector conflicts with cursor")
            }
            Self::History(HistoryQuery {
                initial: Some(HistoryRange::Recent { count }),
                ..
            }) if *count == 0 || *count > 100 => Err("invalid recent history count"),
            Self::CreateThread(create) if create.goal.is_empty() || create.goal.len() > 1024 => {
                Err("invalid thread goal byte bound")
            }
            Self::CreateThread(create) => {
                create.name.as_deref().map_or(Ok(()), validate_thread_name)
            }
            Self::SetThreadName(change) => {
                change.name.as_deref().map_or(Ok(()), validate_thread_name)
            }
            Self::ResolveThread(query) => validate_thread_name(&query.selector),
            Self::Invite(invite) if invite.deadline_millis == Some(0) => {
                Err("deadline must be positive")
            }
            Self::SummarySubmit(submit) if !submit.submission.is_object() => {
                Err("summary submission must be a JSON object")
            }
            Self::HotThreads(query)
                if query.limit == 0 || query.limit > crate::protocol::results::MAX_HOT_THREADS =>
            {
                Err("invalid hot thread limit")
            }
            Self::ParticipantLocations(query)
                if query.seats.is_empty() || query.seats.len() > MAX_BATCH_ITEMS =>
            {
                Err("participant location batch must contain 1..=100 seats")
            }
            Self::AcceptRequired(accept) => accept.validate(),
            Self::Reject(reject) => validate_rejection_reason(&reject.reason),
            Self::ContinuityCheckIn(continuity) => continuity.validate(),
            Self::RecordManagedLaunch(launch) => launch.validate(),
            Self::OperatorOrphanInvite(invite) if invite.deadline_millis == Some(0) => {
                Err("deadline must be positive")
            }
            Self::Search(query)
                if query.max_candidates == 0 || query.max_candidates > MAX_SEARCH_CANDIDATES =>
            {
                Err("invalid search candidate bound")
            }
            Self::Message(query) => query.body.validate(&query.message),
            Self::Diagnostics(query) => {
                if let Some(encoded) = &query.page.cursor
                    && Cursor::decode(encoded)?.scope != CursorScope::Diagnostics
                {
                    return Err("diagnostics cursor scope mismatch");
                }
                Ok(())
            }
            Self::Warnings(query) => {
                if let Some(encoded) = &query.page.cursor
                    && Cursor::decode(encoded)?.scope != CursorScope::Warnings
                {
                    return Err("warnings cursor scope mismatch");
                }
                Ok(())
            }
            Self::ActiveWarnings(query) => {
                if let Some(encoded) = &query.page.cursor
                    && Cursor::decode(encoded)?.scope != CursorScope::ActiveWarnings
                {
                    return Err("active warnings cursor scope mismatch");
                }
                Ok(())
            }
            Self::SendMessage(send) if send.invited_recipients.len() > MAX_BATCH_ITEMS => {
                Err("too many explicit recipients")
            }
            Self::Ack(ack) | Self::AckDisplayed(ack)
                if ack.messages.is_empty() || ack.messages.len() > MAX_BATCH_ITEMS =>
            {
                Err("invalid ack batch size")
            }
            _ => Ok(()),
        }
    }
    pub fn page(&self) -> Option<&PageRequest> {
        match self {
            Self::Directory(v) => Some(&v.page),
            Self::PickerDirectory(v) => Some(&v.page),
            Self::Seats(v) => Some(&v.page),
            Self::SeatInspect(v) => Some(&v.page),
            Self::Inbox(v) | Self::InboxBatch(v) => Some(&v.page),
            Self::Warnings(v) => Some(&v.page),
            Self::ActiveWarnings(v) => Some(&v.page),
            Self::Thread(v) => Some(&v.page),
            Self::History(v) => Some(&v.page),
            Self::Participants(v) => Some(&v.page),
            Self::Recipients(v) => Some(&v.page),
            Self::DeliveryInspect(v) => Some(&v.page),
            Self::PendingReceipts(v) => Some(&v.page),
            Self::LocalIntents(v) => Some(&v.page),
            Self::Search(v) => Some(&v.page),
            Self::Diagnostics(v) => Some(&v.page),
            Self::RetirementJobs(v) => Some(&v.page),
            _ => None,
        }
    }
}

/// Only these commands can cross the local-user operator store boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorCommand {
    Rebind(OperatorRebind),
    FreshSeat(OperatorFreshSeat),
    OrphanInvite(OperatorOrphanInvite),
    Retire(OperatorRetire),
    Replace(OperatorReplace),
}
impl TryFrom<Command> for OperatorCommand {
    type Error = Command;
    fn try_from(command: Command) -> Result<Self, Self::Error> {
        match command {
            Command::OperatorRebind(v) => Ok(Self::Rebind(v)),
            Command::OperatorFreshSeat(v) => Ok(Self::FreshSeat(v)),
            Command::OperatorOrphanInvite(v) => Ok(Self::OrphanInvite(v)),
            Command::OperatorRetire(v) => Ok(Self::Retire(v)),
            Command::OperatorReplace(v) => Ok(Self::Replace(v)),
            other => Err(other),
        }
    }
}

/// Accountable native operations. Ordinary seat resolution and operator actions
/// cannot be passed to the permit-consuming store path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermitMutation {
    CheckIn(CheckIn),
    BeginHandoff(crate::protocol::handoff::HandoffMutation),
    CompleteHandoff(crate::protocol::handoff::HandoffMutation),
    CreateThread(CreateThread),
    Invite(Invite),
    Accept(Accept),
    AcceptRequired(AcceptRequired),
    Reject(Reject),
    SendMessage(SendMessage),
    Ack(Ack),
    AckDisplayed(Ack),
    Leave(Leave),
    SetTopic(SetTopic),
    SetThreadName(SetThreadName),
    Archive(ThreadMutation),
    Reopen(ThreadMutation),
}
impl TryFrom<Command> for PermitMutation {
    type Error = Command;
    fn try_from(command: Command) -> Result<Self, Self::Error> {
        match command {
            Command::CheckIn(v) => Ok(Self::CheckIn(v)),
            Command::BeginHandoff(v) => Ok(Self::BeginHandoff(v)),
            Command::CompleteHandoff(v) => Ok(Self::CompleteHandoff(v)),
            Command::CreateThread(v) => Ok(Self::CreateThread(v)),
            Command::Invite(v) => Ok(Self::Invite(v)),
            Command::Accept(v) => Ok(Self::Accept(v)),
            Command::AcceptRequired(v) => Ok(Self::AcceptRequired(v)),
            Command::Reject(v) => Ok(Self::Reject(v)),
            Command::SendMessage(v) => Ok(Self::SendMessage(v)),
            Command::Ack(v) => Ok(Self::Ack(v)),
            Command::AckDisplayed(v) => Ok(Self::AckDisplayed(v)),
            Command::Leave(v) => Ok(Self::Leave(v)),
            Command::SetTopic(v) => Ok(Self::SetTopic(v)),
            Command::SetThreadName(v) => Ok(Self::SetThreadName(v)),
            Command::Archive(v) => Ok(Self::Archive(v)),
            Command::Reopen(v) => Ok(Self::Reopen(v)),
            other => Err(other),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/protocol/history_full_bodies.rs"]
mod history_full_bodies;
