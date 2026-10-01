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
    Stop(StopRequest),
    ServiceInspect,
    ServiceDisconnect(ServiceDisconnectRequest),
    Directory(DirectoryQuery),
    Seats(SeatsQuery),
    SeatInspect(SeatInspectQuery),
    Inbox(InboxQuery),
    Warnings(WarningsQuery),
    Thread(ThreadQuery),
    History(HistoryQuery),
    Participants(ParticipantsQuery),
    Recipients(RecipientsQuery),
    DeliveryInspect(DeliveryInspectQuery),
    PendingReceipts(PendingReceiptsQuery),
    /// Read-only seat attention digest; never mutates receipts, ACK or checkpoints.
    AttentionDigest(AttentionDigestQuery),
    LocalIntents(LocalIntentsQuery),
    Search(SearchQuery),
    Message(MessageQuery),
    Diagnostics(DiagnosticsQuery),
    OperationStatus(OperationStatusQuery),
    RetirementJobs(RetirementJobsQuery),
    ResolveSeat(ResolveSeat),
    /// TRUST-POLICY C1: seatless resume-only reattachment decision.
    ContinuityCheckIn(ContinuityCheckIn),
    CheckIn(CheckIn),
    /// Person check-in over an agent's binding, as the local account
    /// (TRUST-POLICY A4 override). Human lifecycle only.
    OperatorCheckIn(CheckIn),
    CreateThread(CreateThread),
    Invite(Invite),
    Accept(Accept),
    AcceptRequired(AcceptRequired),
    SendMessage(SendMessage),
    Ack(Ack),
    Leave(Leave),
    SetTopic(SetTopic),
    Archive(ThreadMutation),
    Reopen(ThreadMutation),
    OperatorRebind(OperatorRebind),
    OperatorFreshSeat(OperatorFreshSeat),
    OperatorOrphanInvite(OperatorOrphanInvite),
    OperatorRetire(OperatorRetire),
    OperatorReplace(OperatorReplace),
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
pub struct DirectoryQuery {
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
    /// Optional pane filter: only the nonretired seat(s) mapped to this
    /// target (resolved or unresolved). Absent on the wire when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<HostTargetId>,
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
/// pane's target. It names no seat: the daemon decides the seat, and the
/// caller then performs an ordinary lifecycle check-in for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuityCheckIn {
    pub target: HostTargetId,
    pub harness: crate::protocol::authority::Harness,
    pub native_session: NativeSessionId,
    /// The hook's SessionStart source. Only `resume` can reattach.
    pub source: String,
    pub operation: OperationId,
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
    pub thread: ThreadId,
    pub body: String,
    pub invited_recipients: Vec<SeatId>,
    pub deadline_millis: Option<u64>,
    pub operation: OperationId,
    pub claim: CallerClaim,
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
        if let Some(page) = self.page() {
            page.validate()?;
        }
        let claim = match self {
            Self::CheckIn(v) | Self::OperatorCheckIn(v) => Some(&v.claim),
            Self::CreateThread(v) => Some(&v.claim),
            Self::Invite(v) => Some(&v.claim),
            Self::Accept(v) => Some(&v.claim),
            Self::AcceptRequired(v) => Some(&v.claim),
            Self::SendMessage(v) => Some(&v.claim),
            Self::Ack(v) => Some(&v.claim),
            Self::Leave(v) => Some(&v.claim),
            Self::SetTopic(v) => Some(&v.claim),
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
            Self::Invite(invite) if invite.deadline_millis == Some(0) => {
                Err("deadline must be positive")
            }
            Self::AcceptRequired(accept) => accept.validate(),
            Self::ContinuityCheckIn(continuity) => continuity.validate(),
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
            Self::SendMessage(send) if send.invited_recipients.len() > MAX_BATCH_ITEMS => {
                Err("too many explicit recipients")
            }
            Self::Ack(ack) if ack.messages.is_empty() || ack.messages.len() > MAX_BATCH_ITEMS => {
                Err("invalid ack batch size")
            }
            _ => Ok(()),
        }
    }
    pub fn page(&self) -> Option<&PageRequest> {
        match self {
            Self::Directory(v) => Some(&v.page),
            Self::Seats(v) => Some(&v.page),
            Self::SeatInspect(v) => Some(&v.page),
            Self::Inbox(v) => Some(&v.page),
            Self::Warnings(v) => Some(&v.page),
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
    CreateThread(CreateThread),
    Invite(Invite),
    Accept(Accept),
    AcceptRequired(AcceptRequired),
    SendMessage(SendMessage),
    Ack(Ack),
    Leave(Leave),
    SetTopic(SetTopic),
    Archive(ThreadMutation),
    Reopen(ThreadMutation),
}
impl TryFrom<Command> for PermitMutation {
    type Error = Command;
    fn try_from(command: Command) -> Result<Self, Self::Error> {
        match command {
            Command::CheckIn(v) => Ok(Self::CheckIn(v)),
            Command::CreateThread(v) => Ok(Self::CreateThread(v)),
            Command::Invite(v) => Ok(Self::Invite(v)),
            Command::Accept(v) => Ok(Self::Accept(v)),
            Command::AcceptRequired(v) => Ok(Self::AcceptRequired(v)),
            Command::SendMessage(v) => Ok(Self::SendMessage(v)),
            Command::Ack(v) => Ok(Self::Ack(v)),
            Command::Leave(v) => Ok(Self::Leave(v)),
            Command::SetTopic(v) => Ok(Self::SetTopic(v)),
            Command::Archive(v) => Ok(Self::Archive(v)),
            Command::Reopen(v) => Ok(Self::Reopen(v)),
            other => Err(other),
        }
    }
}
