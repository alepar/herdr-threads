//! Connection-scoped programmatic protocol. These wire types carry requests and
//! diagnostic identities only. Authority comes from the registered socket's
//! server-private handle, never from any field in a decoded frame.

use serde::{Deserialize, Serialize};

use super::{
    ids::{InvitationId, OperationId, RequirementId, SeatId, ServiceAuthorId, ThreadId},
    pagination::{Page, PageRequest},
    results::{MessageKind, MessageSummary},
    time::UtcMillis,
    wire::PROTOCOL_VERSION,
};

pub const SERVICE_SESSION_CAPABILITY: &str = "service_session_v1";

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
                if register.capability != SERVICE_SESSION_CAPABILITY {
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
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;
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
