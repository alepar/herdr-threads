//! Connection-scoped programmatic protocol. These wire types carry requests and
//! diagnostic identities only. Authority comes from the registered socket's
//! server-private handle, never from any field in a decoded frame.

use serde::{Deserialize, Serialize};

use super::{
    commands::{HistoryRange, MAX_BATCH_ITEMS},
    ids::{InvitationId, MessageId, OperationId, RequirementId, SeatId, ServiceAuthorId, ThreadId},
    pagination::{Page, PageRequest},
    results::{DeliveryInspection, MessageKind, MessageSummary},
    time::UtcMillis,
    wire::PROTOCOL_VERSION,
};

pub const SERVICE_SESSION_CAPABILITY: &str = "service_session_v1";
/// The v1 operations plus `Send`, `History` and `Receipts`. A client registers
/// with this name; an old daemon rejects it at register ("unsupported service
/// capability") and there is no silent fallback to v1.
pub const SERVICE_SESSION_CAPABILITY_V2: &str = "service_session_v2";
/// Same bound as native `store::messages::MAX_BODY_BYTES` (protocol must not
/// depend on store; a test pins equality).
pub const SERVICE_SEND_MAX_BODY_BYTES: usize = 65_536;

/// Whether a registering client's capability name is one this build serves.
pub fn is_supported_service_capability(name: &str) -> bool {
    matches!(
        name,
        SERVICE_SESSION_CAPABILITY | SERVICE_SESSION_CAPABILITY_V2
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    Native,
    Programmatic,
    BuiltIn,
}

/// Attribution is durable history, not a credential or live registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum EventAuthor {
    Native(SeatId),
    Programmatic(ServiceAuthorId),
    BuiltIn,
}
impl EventAuthor {
    pub fn kind(&self) -> AuthorKind {
        match self {
            Self::Native(_) => AuthorKind::Native,
            Self::Programmatic(_) => AuthorKind::Programmatic,
            Self::BuiltIn => AuthorKind::BuiltIn,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementState {
    Pending,
    Accepted,
    Released,
    Retired,
}

/// A requirement episode is independent of voluntary membership. A joined
/// native may have a pending requirement confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredMembership {
    pub requirement: RequirementId,
    pub revision: u64,
    pub invitation: InvitationId,
    pub thread: ThreadId,
    pub seat: SeatId,
    pub issuer: ServiceAuthorId,
    pub state: RequirementState,
    pub accepted_by: Option<SeatId>,
    pub accepted_at: Option<UtcMillis>,
}

/// A stale error carries the current state so native callers can reread and
/// explicitly decide whether to accept the new requirement revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequiredAcceptanceError {
    WrongParticipant,
    Stale(RequiredMembership),
    NotPending(RequiredMembership),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvitationConstraint {
    Ordinary,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverity {
    Info,
    Warn,
}

impl From<NotificationSeverity> for MessageKind {
    fn from(value: NotificationSeverity) -> Self {
        match value {
            NotificationSeverity::Info => Self::Info,
            NotificationSeverity::Warn => Self::Warn,
        }
    }
}

/// The first service frame registers. Later frames use Operation on that same
/// connection. Neither frame accepts a claimed author or generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceWireRequest {
    pub version: u16,
    pub request_id: String,
    pub expected_instance: String,
    pub service: ServiceRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "args", rename_all = "snake_case")]
pub enum ServiceRequest {
    Register(ServiceRegister),
    Operation(ServiceOperation),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRegister {
    pub capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "args", rename_all = "snake_case")]
pub enum ServiceOperation {
    EnsureThread(EnsureManagedThread),
    Invite(ServiceInvite),
    Notify(ServiceNotify),
    Membership(ServiceMembershipQuery),
    SetTopic(ServiceSetTopic),
    ReleaseRequirement(ReleaseRequirement),
    Archive(ServiceThreadMutation),
    Reopen(ServiceThreadMutation),
    /// Mutation, journaled (`operation_key` is `Some`). Requires `service_session_v2`.
    Send(ServiceSend),
    /// Query, never journaled. Requires `service_session_v2`.
    History(ServiceHistoryQuery),
    /// Query, never journaled. Requires `service_session_v2`.
    Receipts(ServiceReceiptsQuery),
}

/// Post an ordinary receipt-bearing message into a service-managed thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSend {
    /// Must be managed by this service author and not archived.
    pub thread: ThreadId,
    /// Non-empty, at most `SERVICE_SEND_MAX_BODY_BYTES` bytes.
    pub body: String,
    /// Explicit additions to the joined snapshot: at most 100, no duplicates.
    pub recipients: Vec<SeatId>,
    /// `None` is the instance default receipt duration; `Some` must be positive.
    pub deadline_millis: Option<u64>,
    pub operation: OperationId,
}

/// Read the history of any thread in the instance (managed or not); mirrors
/// the native history query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceHistoryQuery {
    pub thread: ThreadId,
    pub page: PageRequest,
    /// Initial selector; conflicts with a page cursor.
    pub initial: Option<HistoryRange>,
}

/// Read the receipt state of one of this service author's own messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceReceiptsQuery {
    pub message: MessageId,
    pub page: PageRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnsureManagedThread {
    pub thread: ThreadId,
    pub topic: String,
    pub goal: String,
    pub operation: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceInvite {
    pub thread: ThreadId,
    pub seat: SeatId,
    pub constraint: InvitationConstraint,
    pub deadline_millis: Option<u64>,
    pub operation: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceNotify {
    pub thread: ThreadId,
    pub severity: NotificationSeverity,
    pub event_json: serde_json::Value,
    pub operation: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceMembershipQuery {
    pub thread: ThreadId,
    pub seat: Option<SeatId>,
    pub page: PageRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSetTopic {
    pub thread: ThreadId,
    pub topic: String,
    pub operation: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseRequirement {
    pub thread: ThreadId,
    pub seat: SeatId,
    pub requirement: RequirementId,
    pub operation: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceThreadMutation {
    pub thread: ThreadId,
    pub operation: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceWireResponse {
    pub version: u16,
    pub request_id: String,
    pub instance: String,
    pub daemon_boot: String,
    pub result: Result<ServiceResult, super::results::ApiError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum ServiceResult {
    Registered(ServiceRegistration),
    ThreadEnsured(ManagedThread),
    Invitation(ServiceInvitation),
    AlreadyJoined(super::results::AlreadyJoined),
    Notification(ServiceNotification),
    Membership(Page<ServiceMembership>),
    TopicChanged(ManagedThread),
    RequirementReleased(RequiredMembership),
    Archived(ManagedThread),
    Reopened(ManagedThread),
    MessageSent(ServiceMessageSent),
    History(Page<MessageSummary>),
    Receipts(DeliveryInspection),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRegistration {
    pub author: ServiceAuthorId,
    pub daemon_boot: String,
    pub connection_generation: u64,
}

/// The native-compatible message summary retains its optional native seat;
/// this additional ID identifies the durable programmatic author exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceNotification {
    pub summary: MessageSummary,
    pub author: ServiceAuthorId,
}

/// Mirrors `ServiceNotification`: native-compatible summary plus the exact
/// durable author.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceMessageSent {
    pub summary: MessageSummary,
    pub author: ServiceAuthorId,
    /// Receipt obligations created, at least 1. Their live state comes from
    /// `Receipts`, not from this result.
    pub recipient_count: u64,
    pub receipt_duration_millis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedThread {
    pub thread: ThreadId,
    pub owner: ServiceAuthorId,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceInvitation {
    pub invitation: InvitationId,
    pub requirement: Option<RequiredMembership>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceMembership {
    pub thread: ThreadId,
    pub seat: SeatId,
    pub voluntary_state: VoluntaryMembershipState,
    pub requirement: Option<RequiredMembership>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoluntaryMembershipState {
    Absent,
    Invited,
    Joined,
    Left,
    Retired,
}

impl ServiceWireRequest {
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let request: Self = serde_json::from_slice(bytes).map_err(|_| "invalid service frame")?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != PROTOCOL_VERSION {
            return Err("unknown wire version");
        }
        if self.request_id.is_empty()
            || self.request_id.len() > 128
            || !self.request_id.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err("invalid request id");
        }
        if uuid::Uuid::parse_str(&self.expected_instance).map_or(true, |id| {
            id.hyphenated().to_string() != self.expected_instance
        }) {
            return Err("invalid expected instance");
        }
        match &self.service {
            ServiceRequest::Register(register) => {
                if !is_supported_service_capability(&register.capability) {
                    return Err("unsupported service capability");
                }
            }
            ServiceRequest::Operation(operation) => operation.validate()?,
        }
        Ok(())
    }
}

impl ServiceWireResponse {
    pub fn correlates_to(&self, request: &ServiceWireRequest) -> bool {
        self.version == PROTOCOL_VERSION
            && self.request_id == request.request_id
            && self.instance == request.expected_instance
            && uuid::Uuid::parse_str(&self.daemon_boot)
                .is_ok_and(|id| id.hyphenated().to_string() == self.daemon_boot)
            && !matches!(&self.result, Ok(ServiceResult::Registered(registration))
                if registration.daemon_boot != self.daemon_boot || registration.connection_generation == 0)
    }
}

impl ServiceOperation {
    pub fn operation_key(&self) -> Option<&OperationId> {
        match self {
            Self::EnsureThread(request) => Some(&request.operation),
            Self::Invite(request) => Some(&request.operation),
            Self::Notify(request) => Some(&request.operation),
            Self::Membership(_) => None,
            Self::SetTopic(request) => Some(&request.operation),
            Self::ReleaseRequirement(request) => Some(&request.operation),
            Self::Archive(request) | Self::Reopen(request) => Some(&request.operation),
            Self::Send(request) => Some(&request.operation),
            Self::History(_) | Self::Receipts(_) => None,
        }
    }
}

impl ServiceOperation {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::EnsureThread(request) if request.goal.is_empty() || request.goal.len() > 1024 => {
                Err("invalid managed thread goal")
            }
            Self::EnsureThread(request) if request.topic.len() > 1024 => {
                Err("invalid managed thread topic")
            }
            Self::SetTopic(request) if request.topic.len() > 1024 => {
                Err("invalid managed thread topic")
            }
            Self::Invite(request) if request.deadline_millis == Some(0) => {
                Err("deadline must be positive")
            }
            Self::Notify(request)
                if serde_json::to_vec(&request.event_json)
                    .map_or(true, |encoded| encoded.len() > 64 * 1024) =>
            {
                Err("notification event exceeds byte bound")
            }
            Self::Membership(request) => request.page.validate(),
            Self::Send(request) => request.validate(),
            Self::History(request) => request.validate(),
            Self::Receipts(request) => request.page.validate(),
            _ => Ok(()),
        }
    }
}

impl ServiceSend {
    fn validate(&self) -> Result<(), &'static str> {
        if self.body.is_empty() {
            return Err("service message body is empty");
        }
        if self.body.len() > SERVICE_SEND_MAX_BODY_BYTES {
            return Err("service message body exceeds byte bound");
        }
        if self.deadline_millis == Some(0) {
            return Err("deadline must be positive");
        }
        if self.recipients.len() > MAX_BATCH_ITEMS {
            return Err("too many explicit recipients");
        }
        let distinct: std::collections::HashSet<&SeatId> = self.recipients.iter().collect();
        if distinct.len() != self.recipients.len() {
            return Err("duplicate explicit recipient");
        }
        Ok(())
    }
}

impl ServiceHistoryQuery {
    fn validate(&self) -> Result<(), &'static str> {
        if self.initial.is_some() && self.page.cursor.is_some() {
            return Err("history selector conflicts with cursor");
        }
        if let Some(HistoryRange::Recent { count }) = &self.initial
            && (*count == 0 || *count > 100)
        {
            return Err("invalid recent history count");
        }
        self.page.validate()
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    use crate::protocol::pagination::Cursor;
    use crate::protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        commands::{AcceptRequired, Command},
        ids::{ExecutionId, HostTargetId, NativeSessionId},
        wire::{PROTOCOL_VERSION, WireRequest},
    };
    use serde_json::{Value, json};

    const INSTANCE: &str = "123e4567-e89b-12d3-a456-426614174000";
    const BOOT: &str = "123e4567-e89b-12d3-a456-426614174001";

    fn registration() -> ServiceWireRequest {
        ServiceWireRequest {
            version: PROTOCOL_VERSION,
            request_id: "request-1".into(),
            expected_instance: INSTANCE.into(),
            service: ServiceRequest::Register(ServiceRegister {
                capability: SERVICE_SESSION_CAPABILITY.into(),
            }),
        }
    }

    fn send_op(body: &str, recipients: Vec<SeatId>, deadline: Option<u64>) -> ServiceOperation {
        ServiceOperation::Send(ServiceSend {
            thread: ThreadId::new("thread-1"),
            body: body.into(),
            recipients,
            deadline_millis: deadline,
            operation: OperationId::new("send-1"),
        })
    }

    fn seats(count: usize) -> Vec<SeatId> {
        (0..count)
            .map(|n| SeatId::new(format!("seat-{n}")))
            .collect()
    }

    fn history_op(initial: Option<HistoryRange>, page: PageRequest) -> ServiceOperation {
        ServiceOperation::History(ServiceHistoryQuery {
            thread: ThreadId::new("thread-1"),
            page,
            initial,
        })
    }

    fn valid_cursor() -> String {
        use crate::protocol::pagination::{CursorDirection, CursorScope};
        Cursor {
            instance: INSTANCE.into(),
            scope: CursorScope::Directory,
            scope_key: "all".into(),
            filter_digest: "digest".into(),
            direction: CursorDirection::Ascending,
            order_version: 1,
            last_examined_key: None,
            after_ordinal: 1,
            high_water_ordinal: 5,
            scope_revision: Some(1),
            filter_revision: None,
            search: None,
            inbox: None,
            attention: None,
            binding: None,
        }
        .encode()
        .unwrap()
    }

    fn bad_page() -> PageRequest {
        PageRequest {
            limit: 0,
            ..PageRequest::default()
        }
    }

    #[test]
    fn register_accepts_v1_and_v2_only() {
        for capability in [SERVICE_SESSION_CAPABILITY, SERVICE_SESSION_CAPABILITY_V2] {
            let mut request = registration();
            request.service = ServiceRequest::Register(ServiceRegister {
                capability: capability.into(),
            });
            assert_eq!(request.validate(), Ok(()), "{capability}");
        }
        for capability in ["service_session_v3", "", "future-service"] {
            let mut request = registration();
            request.service = ServiceRequest::Register(ServiceRegister {
                capability: capability.into(),
            });
            assert_eq!(
                request.validate(),
                Err("unsupported service capability"),
                "{capability:?}"
            );
        }
    }

    #[test]
    fn operation_keys_are_some_only_for_send_among_new_ops() {
        let key = OperationId::new("send-1");
        assert_eq!(send_op("hi", vec![], None).operation_key(), Some(&key));
        assert_eq!(
            history_op(None, PageRequest::default()).operation_key(),
            None
        );
        let receipts = ServiceOperation::Receipts(ServiceReceiptsQuery {
            message: MessageId::new("m-1"),
            page: PageRequest::default(),
        });
        assert_eq!(receipts.operation_key(), None);
    }

    #[test]
    fn send_validation_rejects_bad_payloads() {
        let max = "a".repeat(SERVICE_SEND_MAX_BODY_BYTES);
        let over = "a".repeat(SERVICE_SEND_MAX_BODY_BYTES + 1);
        let duplicate = vec![SeatId::new("seat-1"), SeatId::new("seat-1")];
        for (op, error) in [
            (send_op("", vec![], None), "service message body is empty"),
            (
                send_op(&over, vec![], None),
                "service message body exceeds byte bound",
            ),
            (send_op("x", vec![], Some(0)), "deadline must be positive"),
            (
                send_op("x", seats(101), None),
                "too many explicit recipients",
            ),
            (
                send_op("x", duplicate, None),
                "duplicate explicit recipient",
            ),
        ] {
            assert_eq!(op.validate(), Err(error));
        }
        for op in [
            send_op("x", vec![], None),
            send_op(&max, vec![], None),
            send_op("x", seats(100), None),
            send_op("x", vec![], Some(1)),
        ] {
            assert_eq!(op.validate(), Ok(()));
        }
    }

    #[test]
    fn history_validation_mirrors_native() {
        let with_cursor = |cursor: String| PageRequest {
            cursor: Some(cursor),
            ..PageRequest::default()
        };
        for count in [0, 101] {
            assert_eq!(
                history_op(Some(HistoryRange::Recent { count }), PageRequest::default()).validate(),
                Err("invalid recent history count")
            );
        }
        assert_eq!(
            history_op(
                Some(HistoryRange::After { sequence: 3 }),
                with_cursor(valid_cursor())
            )
            .validate(),
            Err("history selector conflicts with cursor")
        );
        assert_eq!(
            history_op(None, bad_page()).validate(),
            Err("invalid page limit")
        );
        for op in [
            history_op(
                Some(HistoryRange::Recent { count: 100 }),
                PageRequest::default(),
            ),
            history_op(
                Some(HistoryRange::After { sequence: 3 }),
                PageRequest::default(),
            ),
            history_op(None, with_cursor(valid_cursor())),
        ] {
            assert_eq!(op.validate(), Ok(()));
        }
    }

    #[test]
    fn receipts_validation_checks_page() {
        let receipts = |page| {
            ServiceOperation::Receipts(ServiceReceiptsQuery {
                message: MessageId::new("m-1"),
                page,
            })
        };
        assert_eq!(receipts(bad_page()).validate(), Err("invalid page limit"));
        assert_eq!(receipts(PageRequest::default()).validate(), Ok(()));
    }

    #[test]
    fn send_frame_rejects_forged_authority_fields() {
        let request = ServiceWireRequest {
            version: PROTOCOL_VERSION,
            request_id: "request-2".into(),
            expected_instance: INSTANCE.into(),
            service: ServiceRequest::Operation(send_op("hi", seats(1), None)),
        };
        let decodes =
            |value: &Value| ServiceWireRequest::decode(&serde_json::to_vec(value).unwrap());
        let mut frame = serde_json::to_value(&request).unwrap();
        assert_eq!(decodes(&frame).unwrap(), request);
        assert_eq!(frame["service"]["kind"], "operation");
        assert_eq!(frame["service"]["args"]["kind"], "send");
        frame["service"]["args"]["args"]["author"] = json!("graph");
        assert!(decodes(&frame).is_err());
        frame["service"]["args"]["args"]
            .as_object_mut()
            .unwrap()
            .remove("author");
        frame["service"]["args"]["args"]["connection_generation"] = json!(3);
        assert!(decodes(&frame).is_err());
        frame["service"]["args"]["args"]
            .as_object_mut()
            .unwrap()
            .remove("connection_generation");
        assert!(decodes(&frame).is_ok());
        frame["author"] = json!("graph");
        assert!(decodes(&frame).is_err());
    }

    #[test]
    fn new_results_roundtrip() {
        let summary = json!({
            "message": "m-1", "thread": "t-1", "author": null,
            "event_author": {"kind": "programmatic", "id": "graph"},
            "kind": "ordinary", "sequence": 4, "created_at": 1,
            "actor_label": "herdr-graph", "preview_data": "do it",
            "preview_omitted": false, "preview_detail_argv": null
        });
        let page = |items: Value| {
            json!({"items": items, "next_cursor": null, "next_argv": null,
                "high_water_ordinal": 0, "scope_revision": null, "has_more": false,
                "stop_reason": "complete", "consistency": "bounded_live"})
        };
        let sent = json!({"kind": "message_sent", "data": {
            "summary": summary, "author": "graph", "recipient_count": 2,
            "receipt_duration_millis": 60000}});
        let history = json!({"kind": "history", "data": page(json!([summary]))});
        let receipts = json!({"kind": "receipts", "data": {
            "message": summary,
            "delivery": {"committed": 1, "attempted": null, "submitted": null,
                "read": null, "acknowledged": 0},
            "recipients": page(json!([]))}});
        for fixture in [sent.clone(), history, receipts] {
            let parsed: ServiceResult = serde_json::from_value(fixture.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), fixture);
        }
        let mut extra = sent;
        extra["data"]["recipients"] = json!([]);
        assert!(serde_json::from_value::<ServiceResult>(extra).is_err());
    }

    #[test]
    fn send_body_bound_matches_native() {
        assert_eq!(
            SERVICE_SEND_MAX_BODY_BYTES,
            crate::store::messages::MAX_BODY_BYTES
        );
    }

    #[test]
    fn ordinary_and_service_envelopes_are_disjoint() {
        let ordinary = json!({
            "version": PROTOCOL_VERSION,
            "request_id": "request-1",
            "expected_instance": INSTANCE,
            "command": {"kind": "health"}
        });
        let service = serde_json::to_value(registration()).unwrap();
        assert!(WireRequest::decode(&serde_json::to_vec(&ordinary).unwrap()).is_ok());
        assert!(ServiceWireRequest::decode(&serde_json::to_vec(&ordinary).unwrap()).is_err());
        assert!(ServiceWireRequest::decode(&serde_json::to_vec(&service).unwrap()).is_ok());
        assert!(WireRequest::decode(&serde_json::to_vec(&service).unwrap()).is_err());

        let mut forged = service;
        forged["author"] = json!("system");
        assert!(ServiceWireRequest::decode(&serde_json::to_vec(&forged).unwrap()).is_err());
        forged.as_object_mut().unwrap().remove("author");
        forged["connection_generation"] = json!(7);
        assert!(ServiceWireRequest::decode(&serde_json::to_vec(&forged).unwrap()).is_err());
    }

    #[test]
    fn registration_identity_cannot_be_replayed_as_authority() {
        let request = registration();
        let response = ServiceWireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id.clone(),
            instance: request.expected_instance.clone(),
            daemon_boot: BOOT.into(),
            result: Ok(ServiceResult::Registered(ServiceRegistration {
                author: ServiceAuthorId::new("graph"),
                daemon_boot: BOOT.into(),
                connection_generation: 3,
            })),
        };
        assert!(response.correlates_to(&request));
        let mut forged: Value = serde_json::to_value(&request).unwrap();
        forged["service"]["args"]["author"] = json!("graph");
        forged["service"]["args"]["connection_generation"] = json!(3);
        assert!(ServiceWireRequest::decode(&serde_json::to_vec(&forged).unwrap()).is_err());
        let mut wrong_boot = response;
        if let Ok(ServiceResult::Registered(registration)) = &mut wrong_boot.result {
            registration.daemon_boot = INSTANCE.into();
        }
        assert!(!wrong_boot.correlates_to(&request));
    }

    #[test]
    fn native_required_acceptance_reaches_wire_and_distinct_permit_payload() {
        let frame = json!({
            "version": PROTOCOL_VERSION,
            "request_id": "required-1",
            "expected_instance": INSTANCE,
            "command": {"kind": "accept_required", "args": {
                "thread": "thread-1", "invitation": "invite-1",
                "requirement": "requirement-1", "expected_revision": 4,
                "operation": "accept-1", "claim": {
                    "instance": INSTANCE, "seat": "seat-1", "binding_generation": 1,
                    "role": "top_level", "harness": "codex", "native_session": "native-1",
                    "execution": "execution-1", "target": "target-1"
                }
            }}
        });
        let decoded = WireRequest::decode(&serde_json::to_vec(&frame).unwrap()).unwrap();
        let mut zero_revision = frame.clone();
        zero_revision["command"]["args"]["expected_revision"] = json!(0);
        assert!(WireRequest::decode(&serde_json::to_vec(&zero_revision).unwrap()).is_err());
        let mut subagent = frame.clone();
        subagent["command"]["args"]["claim"]["role"] = json!("subagent");
        assert!(WireRequest::decode(&serde_json::to_vec(&subagent).unwrap()).is_err());

        let mut ordinary = frame.clone();
        ordinary["command"]["kind"] = json!("accept");
        let args = ordinary["command"]["args"].as_object_mut().unwrap();
        args.remove("invitation");
        args.remove("requirement");
        args.remove("expected_revision");
        let ordinary = WireRequest::decode(&serde_json::to_vec(&ordinary).unwrap()).unwrap();
        assert!(matches!(
            crate::protocol::commands::PermitMutation::try_from(ordinary.command),
            Ok(crate::protocol::commands::PermitMutation::Accept(_))
        ));

        let encoded = serde_json::to_vec(&decoded).unwrap();
        let roundtrip = WireRequest::decode(&encoded).unwrap();
        let Command::AcceptRequired(required) = roundtrip.command else {
            panic!("required acceptance changed command kind");
        };
        assert_eq!(required.expected_revision, 4);
        let first = crate::store::cooperative_permit_request(
            &crate::protocol::commands::PermitMutation::try_from(Command::AcceptRequired(
                required.clone(),
            ))
            .unwrap(),
        )
        .unwrap();
        let mut later = required.clone();
        later.expected_revision = 5;
        let changed_revision = crate::store::cooperative_permit_request(
            &crate::protocol::commands::PermitMutation::try_from(Command::AcceptRequired(later))
                .unwrap(),
        )
        .unwrap();
        assert_ne!(first.payload_hash, changed_revision.payload_hash);
        let mut later_episode = required;
        later_episode.requirement = RequirementId::new("requirement-2");
        let changed_episode = crate::store::cooperative_permit_request(
            &crate::protocol::commands::PermitMutation::try_from(Command::AcceptRequired(
                later_episode,
            ))
            .unwrap(),
        )
        .unwrap();
        assert_ne!(first.payload_hash, changed_episode.payload_hash);
    }

    #[test]
    fn notification_result_roundtrips_durable_author_not_just_label() {
        let message = json!({
            "message": "notice-1", "thread": "thread-1", "author": null,
            "kind": "warn", "sequence": 1, "created_at": 1,
            "actor_label": "graph", "preview_data": "notice", "preview_omitted": false,
            "preview_detail_argv": null
        });
        let result = |author: &str| {
            json!({
                "kind": "notification", "data": {
                    "summary": message,
                    "author": author
                }
            })
        };
        let first: ServiceResult = serde_json::from_value(result("service-a")).unwrap();
        let second: ServiceResult = serde_json::from_value(result("service-b")).unwrap();
        assert_ne!(first, second);
        assert_eq!(serde_json::to_value(first).unwrap(), result("service-a"));
        assert_eq!(serde_json::to_value(second).unwrap(), result("service-b"));
    }

    #[test]
    fn required_acceptance_must_match_the_observed_episode_and_revision() {
        let current = RequiredMembership {
            requirement: RequirementId::new("requirement-2"),
            revision: 2,
            invitation: InvitationId::new("invitation-1"),
            thread: ThreadId::new("thread-1"),
            seat: SeatId::new("seat-1"),
            issuer: ServiceAuthorId::new("graph"),
            state: RequirementState::Pending,
            accepted_by: None,
            accepted_at: None,
        };
        let mut accept = AcceptRequired {
            thread: current.thread.clone(),
            invitation: current.invitation.clone(),
            requirement: current.requirement.clone(),
            expected_revision: 1,
            operation: OperationId::new("accept-1"),
            claim: CallerClaim {
                instance: INSTANCE.into(),
                seat: current.seat.clone(),
                binding_generation: 1,
                role: CallerRole::TopLevel,
                harness: Harness::Codex,
                native_session: NativeSessionId::new("native-1"),
                execution: ExecutionId::new("execution-1"),
                target: HostTargetId::new("target-1"),
            },
        };
        assert_eq!(
            accept.check_current(&current),
            Err(RequiredAcceptanceError::Stale(current.clone()))
        );
        accept.expected_revision = 2;
        assert_eq!(accept.check_current(&current), Ok(()));
        let mut replacement = current.clone();
        replacement.requirement = RequirementId::new("requirement-3");
        assert!(matches!(
            accept.check_current(&replacement),
            Err(RequiredAcceptanceError::Stale(_))
        ));
        replacement = current.clone();
        replacement.state = RequirementState::Released;
        assert!(matches!(
            accept.check_current(&replacement),
            Err(RequiredAcceptanceError::NotPending(_))
        ));
        let ordinary = Command::Accept(crate::protocol::commands::Accept {
            thread: accept.thread,
            operation: accept.operation,
            claim: accept.claim,
        });
        assert_eq!(ordinary.validate(), Ok(()));
    }
}
