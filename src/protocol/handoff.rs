//! Internal compound lifecycle contract. These claims protect work; they are
//! never receipts or evidence that a native agent was started.
use super::{
    authority::CallerClaim,
    ids::{OperationId, SeatId, ThreadId},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffIdentity {
    pub compound: OperationId,
    /// Original frozen journal semantic SHA-256, unchanged across upgrades.
    pub digest: String,
    pub claim: CallerClaim,
    pub thread: Option<ThreadId>,
    pub recipient: SeatId,
    pub create_key: OperationId,
    pub invite_key: OperationId,
    pub send_key: OperationId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffMutation {
    pub identity: HandoffIdentity,
    pub operation: OperationId,
}
/// Capability-gated delivery envelope. Old phase requests and replay bytes remain unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryMutation {
    pub scope: crate::cli::journal::IntentScope,
    pub claim: CallerClaim,
    pub digest: String,
    pub plan: crate::cli::journal::DeliveryPlan,
    pub action: DeliveryAction,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "phase",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DeliveryAction {
    Status(HandoffIdentity),
    Prepare(HandoffIdentity),
    Begin(HandoffMutation),
    Create(crate::protocol::commands::CreateThread),
    Invite(crate::protocol::commands::Invite),
    Send(crate::protocol::commands::SendMessage),
    Complete(HandoffMutation),
}
impl DeliveryMutation {
    pub fn identity(&self) -> HandoffIdentity {
        let keys = &self.plan.payload.keys;
        HandoffIdentity {
            compound: keys.compound.clone(),
            digest: self.digest.clone(),
            claim: self.claim.clone(),
            thread: self.plan.payload.channel.thread().cloned(),
            recipient: self.plan.recipient.clone(),
            create_key: keys.create.clone(),
            invite_key: keys.invite.clone(),
            send_key: keys.send.clone(),
        }
    }
    pub fn inner(&self) -> Result<crate::protocol::commands::PermitMutation, &'static str> {
        use crate::protocol::commands::PermitMutation;
        Ok(match &self.action {
            DeliveryAction::Status(_) | DeliveryAction::Prepare(_) => {
                return Err("delivery query has no mutation permit");
            }
            DeliveryAction::Begin(v) => PermitMutation::BeginHandoff(v.clone()),
            DeliveryAction::Create(v) => PermitMutation::CreateThread(v.clone()),
            DeliveryAction::Invite(v) => PermitMutation::Invite(v.clone()),
            DeliveryAction::Send(v) => PermitMutation::SendMessage(v.clone()),
            DeliveryAction::Complete(v) => PermitMutation::CompleteHandoff(v.clone()),
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        use crate::cli::journal::{OriginalActor, SemanticMutation, classify_original_actor};
        use sha2::{Digest, Sha256};
        self.plan.validate().map_err(|_| "invalid delivery plan")?;
        let semantic = SemanticMutation::Frozen {
            claim: self.claim.clone(),
            mutation: Box::new(SemanticMutation::HandoffDelivery(Box::new(
                self.plan.clone(),
            ))),
        };
        if classify_original_actor(&self.scope, &semantic)
            .map_err(|_| "invalid delivery original actor")?
            != OriginalActor::Agent
            || self.claim.role != super::authority::CallerRole::TopLevel
            || self.claim.instance != self.plan.payload.namespace.instance
            || self.digest
                != format!(
                    "{:x}",
                    Sha256::digest(
                        serde_json::to_vec(&semantic).map_err(|_| "invalid delivery semantic")?
                    )
                )
        {
            return Err("delivery original identity mismatch");
        }
        let keys = &self.plan.payload.keys;
        let thread_matches = |thread: &ThreadId| {
            self.plan
                .payload
                .channel
                .thread()
                .is_none_or(|saved| saved == thread)
        };
        let matches = match &self.action {
            DeliveryAction::Status(v) | DeliveryAction::Prepare(v) => *v == self.identity(),
            DeliveryAction::Begin(v) => v.identity == self.identity() && v.operation == keys.begin,
            DeliveryAction::Complete(v) => {
                v.identity == self.identity() && v.operation == keys.complete
            }
            DeliveryAction::Create(v) => {
                v.claim == self.claim
                    && v.operation == keys.create
                    && matches!(&self.plan.payload.channel, HandoffChannel::New { name, topic, goal } if name == &v.name && topic == &v.topic && goal == &v.goal)
            }
            DeliveryAction::Invite(v) => {
                v.claim == self.claim
                    && v.operation == keys.invite
                    && v.seat == self.plan.recipient
                    && v.deadline_millis.is_none()
                    && thread_matches(&v.thread)
            }
            DeliveryAction::Send(v) => {
                v.delivery_mode == crate::protocol::commands::DeliveryMode::Ordinary
                    && v.claim == self.claim
                    && v.operation == keys.send
                    && v.body == self.plan.payload.body
                    && v.invited_recipients == [self.plan.recipient.clone()]
                    && v.deadline_millis.is_none()
                    && !v.relays_user
                    && v.user_intent.is_none()
                    && thread_matches(&v.thread)
            }
        };
        if !matches {
            return Err("delivery phase differs from frozen original");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffState {
    Live,
    Completed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffResult {
    pub compound: OperationId,
    pub thread: Option<ThreadId>,
    pub state: HandoffState,
}

/// Additive v1 namespace. Frozen paths are compared, never normalized on replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffNamespace {
    pub instance: String,
    pub state_dir: std::path::PathBuf,
    pub host_endpoint: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HandoffChannel {
    Existing {
        thread: ThreadId,
    },
    New {
        name: Option<String>,
        topic: String,
        goal: String,
    },
}
impl HandoffChannel {
    pub fn thread(&self) -> Option<&ThreadId> {
        match self {
            Self::Existing { thread } => Some(thread),
            Self::New { .. } => None,
        }
    }
}

/// Distinct immutable children; attempt children derive from `compound` and N.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffKeys {
    pub compound: OperationId,
    pub begin: OperationId,
    pub create: OperationId,
    pub invite: OperationId,
    pub send: OperationId,
    pub complete: OperationId,
}

/// Shared immutable staging payload; delivery carries no launch requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffPayload {
    pub namespace: HandoffNamespace,
    pub keys: HandoffKeys,
    pub channel: HandoffChannel,
    pub body: String,
}

/// Normalized native arguments before topology exists. No caller pane/env.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapLaunch {
    pub harness: crate::protocol::authority::Harness,
    pub binary: Option<String>,
    pub name: Option<String>,
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapPayload {
    pub handoff: HandoffPayload,
    pub workspace: HostTargetId,
    pub cwd: std::path::PathBuf,
    pub label: String,
    pub focus: bool,
    pub env: std::collections::BTreeMap<String, String>,
    pub launch: BootstrapLaunch,
    pub handoff_key: OperationId,
    pub resolve_key: OperationId,
    pub attach_key: OperationId,
    pub linked_complete_key: OperationId,
}

/// A positive creation attempt. Status never carries submission authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BootstrapAttempt(std::num::NonZeroU32);
impl BootstrapAttempt {
    pub fn new(value: u32) -> Result<Self, &'static str> {
        std::num::NonZeroU32::new(value)
            .map(Self)
            .ok_or("attempt must be positive")
    }
    pub fn first() -> Self {
        Self(std::num::NonZeroU32::MIN)
    }
    pub fn get(self) -> u32 {
        self.0.get()
    }
    /// Bounded deterministic children; no self-hashed identity envelope.
    pub fn operation(
        self,
        compound: &OperationId,
        phase: &str,
    ) -> Result<OperationId, &'static str> {
        if !matches!(phase, "reserve" | "record" | "check" | "not_submitted") {
            return Err("invalid attempt phase");
        }
        use sha2::{Digest, Sha256};
        OperationId::parse(format!(
            "bootstrap-{:x}",
            Sha256::digest(format!("{}\0{}\0{phase}", compound.as_str(), self.get()).as_bytes())
        ))
    }
}

/// Full immutable payload plus original journal scope and claim. `digest` hashes
/// the semantic journal payload, not this envelope (which includes that digest).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapIdentity {
    pub compound: OperationId,
    pub scope: crate::cli::journal::IntentScope,
    pub claim: CallerClaim,
    pub digest: String,
    pub payload: BootstrapPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapState {
    Prepared,
    PossibleCreation,
    Created,
    Attached,
    Completed,
    Cancelled,
}

/// Per-attempt lifecycle is separate from absorbing parent dispositions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapAttemptState {
    Prepared,
    PossibleCreation,
    NotSubmitted,
    OutcomeUnknown,
    Created,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapAttachment {
    pub attempt: BootstrapAttempt,
    pub created: crate::ports::CreatedTab,
    pub resolve_operation: OperationId,
    pub resolved_seat: SeatId,
    pub handoff: HandoffIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginBootstrap {
    pub identity: BootstrapIdentity,
    pub operation: OperationId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReserveBootstrapAttempt {
    pub identity: BootstrapIdentity,
    pub operation: OperationId,
    pub expected_attempt: BootstrapAttempt,
}
/// Cooperative original-caller report of the actual typed transport's zero-byte
/// branch, never a human assertion or an error-code inference. Public dispatch
/// stays inert until the real producer and canonical guards are integrated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordBootstrapNotSubmitted {
    pub identity: BootstrapIdentity,
    pub operation: OperationId,
    pub expected_attempt: BootstrapAttempt,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordBootstrapCreated {
    pub identity: BootstrapIdentity,
    pub operation: OperationId,
    pub expected_attempt: BootstrapAttempt,
    pub evidence: crate::ports::CreatedTab,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachBootstrapHandoff {
    pub identity: BootstrapIdentity,
    pub operation: OperationId,
    pub attachment: BootstrapAttachment,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckBootstrapSubmission {
    pub identity: BootstrapIdentity,
    pub operation: OperationId,
    pub expected_attempt: BootstrapAttempt,
    /// Canonical decision revision frozen by reservation; freshly compare in A2.
    pub expected_administrative_revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapStatus {
    pub identity: BootstrapIdentity,
}

/// Bootstrap-only current exact-tab resolution, before ordinary allocation.
/// Target and scope are obtained from canonical recorded creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveBootstrapSeat {
    pub identity: BootstrapIdentity,
    pub expected_attempt: BootstrapAttempt,
    pub operation: OperationId,
}
impl ResolveBootstrapSeat {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.identity.validate()?;
        if self.operation != self.identity.payload.resolve_key {
            return Err("bootstrap resolve key differs");
        }
        Ok(())
    }
}

/// Only the first transition transaction returns this authorization. It is
/// deliberately absent from BootstrapResult/status and completed replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapSubmissionAuthorization {
    pub compound: OperationId,
    pub attempt: BootstrapAttempt,
    pub reservation: OperationId,
    pub administrative_revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReserveBootstrapResult {
    Authorized {
        authorization: BootstrapSubmissionAuthorization,
    },
    Replay {
        status: Box<BootstrapResult>,
    },
}
/// Continues an already reserved attempt; grants no second submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapSubmissionChecked {
    pub compound: OperationId,
    pub attempt: BootstrapAttempt,
    pub administrative_revision: u64,
}

/// Operator assertion, not evidence of launch or a replacement original claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BootstrapRecoveryDisposition {
    CreatedPane {
        evidence: crate::ports::CreatedTab,
        /// Historical operator-assertion correlation, retained byte-for-byte.
        /// Fresh canonical admission uses an independently generated read ID.
        structural_reference: HostCallId,
    },
    NotCreated {
        quiescence: BootstrapQuiescenceAssertion,
    },
    Cancelled {
        reason: String,
        quiescence: BootstrapQuiescenceAssertion,
        child_guard: BootstrapCancellationGuard,
    },
}
/// An explicit inspected assertion. Canonical code must hold the normal
/// operation lock; this value can never prove quiescence or grant authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapQuiescenceAssertion {
    InspectedNoncreationAndQuiescence,
    InspectedQuiescence,
}
/// Storage MUST prove no live exact child, including legacy hints, in its
/// deciding transaction. An absent child supplied by a caller is no such proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapCancellationGuard {
    pub attached_child: Option<HandoffIdentity>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverBootstrap {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspection: Option<BootstrapRecoveryInspection>,
    pub identity: BootstrapIdentity,
    pub expected_attempt: BootstrapAttempt,
    pub operation: OperationId,
    pub disposition: BootstrapRecoveryDisposition,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapRecoveryResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspection: Option<BootstrapRecoveryInspection>,
    pub identity: BootstrapIdentity,
    pub attempt: BootstrapAttempt,
    pub operation: OperationId,
    pub disposition: BootstrapRecoveryDisposition,
    /// Derived from authenticated local-account peer, never a caller claim.
    pub operator_uid: u32,
    pub operator_provenance: String,
    pub creation: Option<crate::ports::CreatedTab>,
    pub state: BootstrapState,
}

/// Cooperative retained launcher report; never an execution attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedBootstrapReport {
    pub thread: ThreadId,
    pub recipient: SeatId,
    pub pane: HostTargetId,
    pub kind: String,
    pub launch: BootstrapLaunch,
    pub report: serde_json::Value,
    pub report_digest: String,
    pub terminal: TerminalId,
    pub host_incarnation: HostBootId,
}
/// One deciding transaction MUST validate both fences, exact attachment,
/// namespace/original actor, legacy completion requirements and retained report,
/// then record unchanged child semantic identity and complete both atomically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompleteLinkedBootstrap {
    pub identity: BootstrapIdentity,
    pub attachment: BootstrapAttachment,
    pub operation: OperationId,
    pub legacy_completion: HandoffMutation,
    pub retained: LinkedBootstrapReport,
}
/// Historical presentation after original-origin/namespace validation only;
/// missing/corrupt retained report refuses, never reopens either fence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedBootstrapResult {
    pub identity: BootstrapIdentity,
    pub attachment: BootstrapAttachment,
    pub retained: LinkedBootstrapReport,
    pub legacy_result: HandoffResult,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapResult {
    pub compound: OperationId,
    pub attempt: BootstrapAttempt,
    pub attempt_state: BootstrapAttemptState,
    pub state: BootstrapState,
    pub creation: Option<crate::ports::CreatedTab>,
    pub attachment: Option<BootstrapAttachment>,
    pub completed: Option<Box<CompletedBootstrapResult>>,
    pub recovery: Option<Box<BootstrapRecoveryResult>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryParticipation {
    Joined,
    InvitedPending,
    StagedUnbound,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryResult {
    pub compound: OperationId,
    pub thread: ThreadId,
    pub recipient: SeatId,
    pub participation: DeliveryParticipation,
    pub state: HandoffState,
}

use super::ids::{HostBootId, HostCallId, HostTargetId, TerminalId};
// Pure contract validation; authoritative cross-record guards belong to storage.
pub(crate) fn bounded_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}
pub(crate) fn frozen_absolute_path(path: &std::path::Path) -> bool {
    path.is_absolute()
        && path.to_str().is_some_and(|s| {
            s.len() <= 4096
                && !s.contains('\0')
                && !s.split('/').any(|part| matches!(part, "." | ".."))
        })
}
pub(crate) fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl HandoffNamespace {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !bounded_text(&self.instance, 128)
            || self.instance.chars().any(char::is_control)
            || !frozen_absolute_path(&self.state_dir)
            || !frozen_absolute_path(&self.host_endpoint)
            || self.host_endpoint.file_name().is_none()
        {
            return Err("invalid frozen handoff namespace");
        }
        Ok(())
    }
}
impl HandoffPayload {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.namespace.validate()?;
        if !bounded_text(&self.body, 1024) {
            return Err("invalid handoff body");
        }
        if let HandoffChannel::New { name, topic, goal } = &self.channel {
            if !bounded_text(topic, 1024) || !bounded_text(goal, 1024) {
                return Err("invalid new channel text");
            }
            if let Some(name) = name {
                super::commands::validate_thread_name(name)?;
            }
        }
        let k = &self.keys;
        let keys = [
            &k.compound,
            &k.begin,
            &k.create,
            &k.invite,
            &k.send,
            &k.complete,
        ];
        if keys
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != keys.len()
        {
            return Err("duplicate compound child key");
        }
        Ok(())
    }
}
impl BootstrapLaunch {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.harness == super::authority::Harness::Human
            || self.binary.as_ref().is_some_and(|b| !bounded_text(b, 4096))
            || self
                .name
                .as_ref()
                .is_some_and(|n| !bounded_text(n, 128) || n.chars().any(char::is_control))
            || self.argv.len() > 256
            || self.argv.iter().any(|a| a.len() > 4096 || a.contains('\0'))
            || self.argv.iter().map(String::len).sum::<usize>() > 65536
        {
            return Err("invalid frozen bootstrap launch arguments");
        }
        Ok(())
    }
}
impl BootstrapPayload {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.handoff.validate()?;
        self.launch.validate()?;
        if !frozen_absolute_path(&self.cwd)
            || !bounded_text(&self.label, 128)
            || self.label.chars().any(char::is_control)
            || self.focus
            || !self.env.is_empty()
        {
            return Err("invalid bootstrap topology payload");
        }
        let k = &self.handoff.keys;
        let keys = [
            &k.compound,
            &k.begin,
            &k.create,
            &k.invite,
            &k.send,
            &k.complete,
            &self.handoff_key,
            &self.resolve_key,
            &self.attach_key,
            &self.linked_complete_key,
        ];
        if keys
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != keys.len()
        {
            return Err("duplicate bootstrap child key");
        }
        Ok(())
    }
}
impl BootstrapIdentity {
    /// Hash the original immutable semantic payload, excluding this envelope.
    pub fn semantic_digest(&self) -> Result<String, &'static str> {
        use sha2::{Digest, Sha256};
        let semantic = crate::cli::journal::SemanticMutation::Frozen {
            claim: self.claim.clone(),
            mutation: Box::new(crate::cli::journal::SemanticMutation::HandoffBootstrap(
                Box::new(crate::cli::journal::BootstrapPlan {
                    version: 1,
                    payload: self.payload.clone(),
                }),
            )),
        };
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&semantic).map_err(|_| "invalid bootstrap semantic payload")?
            )
        ))
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.payload.validate()?;
        if self.compound != self.payload.handoff.keys.compound
            || !valid_digest(&self.digest)
            || self.digest != self.semantic_digest()?
            || self.claim.role != super::authority::CallerRole::TopLevel
            || self.claim.instance != self.payload.handoff.namespace.instance
            || !matches!(&self.scope, crate::cli::journal::IntentScope::Cooperative { instance, seat } if instance == &self.claim.instance && seat == &self.claim.seat)
        {
            return Err("bootstrap original identity mismatch");
        }
        Ok(())
    }
}
/// Immutable canonical inspected-state binding. It is an operator assertion,
/// never authority or evidence of transport zero submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapRecoveryInspection {
    pub version: u32,
    pub digest: String,
}
impl BootstrapRecoveryInspection {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 || !valid_digest(&self.digest) {
            return Err("invalid bootstrap recovery inspection");
        }
        Ok(())
    }
    /// Only the bounded canonical tuple is hashed. Completed reports are omitted;
    /// terminal state already forbids fresh mutation. Prior recovery is finite.
    pub fn from_status(status: &BootstrapResult) -> Result<Self, &'static str> {
        use sha2::{Digest, Sha256};
        struct Hasher {
            hash: Sha256,
            remaining: usize,
        }
        impl std::io::Write for Hasher {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.remaining = self
                    .remaining
                    .checked_sub(bytes.len())
                    .ok_or_else(|| std::io::Error::other("oversized recovery inspection"))?;
                self.hash.update(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        // Creation + attachment + latest recovery have 128/128/256 KiB caps.
        // Allow bounded tuple tags and scalar fields, never a completed report.
        let mut writer = Hasher {
            hash: Sha256::new(),
            remaining: 512 * 1024 + 1024,
        };
        serde_json::to_writer(
            &mut writer,
            &(
                "handoff.bootstrap.inspection.v1",
                &status.compound,
                status.attempt,
                status.attempt_state,
                status.state,
                &status.creation,
                &status.attachment,
                &status.recovery,
            ),
        )
        .map_err(|_| "invalid or oversized recovery inspection snapshot")?;
        Ok(Self {
            version: 1,
            digest: format!("{:x}", writer.hash.finalize()),
        })
    }
}
impl RecoverBootstrap {
    pub fn decision_operation(&self) -> Result<OperationId, &'static str> {
        use sha2::{Digest, Sha256};
        let bytes = if let Some(inspection) = &self.inspection {
            inspection.validate()?;
            serde_json::to_vec(&(
                "handoff.bootstrap.recovery-key.v1",
                &self.identity.compound,
                &self.identity.digest,
                self.expected_attempt,
                &self.disposition,
                inspection,
            ))
        } else {
            // Preserve the exact historical four-tuple and absent-field bytes.
            serde_json::to_vec(&(
                &self.identity.compound,
                &self.identity.digest,
                self.expected_attempt,
                &self.disposition,
            ))
        }
        .map_err(|_| "invalid recovery assertion")?;
        OperationId::parse(format!("bootstrap-recovery-{:x}", Sha256::digest(bytes)))
    }
}

impl BootstrapRecoveryDisposition {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::CreatedPane { evidence, .. } => evidence.validate(),
            Self::NotCreated {
                quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
            } => Ok(()),
            Self::NotCreated { .. } => {
                Err("not-created must assert inspected noncreation and quiescence")
            }
            Self::Cancelled { reason, .. } if !bounded_text(reason, 4096) => {
                Err("cancellation reason must be nonblank and at most 4096 UTF-8 bytes")
            }
            Self::Cancelled { .. } => Ok(()),
        }
    }
}
impl LinkedBootstrapReport {
    pub fn validate(&self) -> Result<(), &'static str> {
        use sha2::{Digest, Sha256};
        self.launch.validate()?;
        let expected_digest = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&self.report).map_err(|_| "invalid retained report")?
            )
        );
        if self.kind != "launch"
            || self.report_digest != expected_digest
            || !self.report.is_object()
            || self
                .report
                .get("outcome")
                .and_then(serde_json::Value::as_str)
                != Some("started")
            || self.report.get("pane").and_then(serde_json::Value::as_str)
                != Some(self.pane.as_str())
            || self.report.get("seat").and_then(serde_json::Value::as_str)
                != Some(self.recipient.as_str())
            || self
                .report
                .get("harness")
                .and_then(serde_json::Value::as_str)
                != Some(self.launch.harness.as_str())
            || !self
                .report
                .get("argv")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|argv| {
                    !argv.is_empty()
                        && argv.len() <= 256
                        && argv.iter().all(|v| {
                            v.as_str()
                                .is_some_and(|s| s.len() <= 4096 && !s.contains('\0'))
                        })
                })
            || serde_json::to_vec(&self.report)
                .map_err(|_| "invalid retained report")?
                .len()
                > 1024 * 1024
        {
            return Err("invalid retained successful launch report");
        }
        Ok(())
    }
}
impl BootstrapAttachment {
    pub fn validate(&self, identity: &BootstrapIdentity) -> Result<(), &'static str> {
        self.created.validate()?;
        if self.resolve_operation != identity.payload.resolve_key
            || self.created.workspace != identity.payload.workspace
            || self.created.witness.endpoint != identity.payload.handoff.namespace.host_endpoint
            || self.handoff.compound != identity.payload.handoff_key
            || self.resolved_seat != self.handoff.recipient
            || self.handoff.claim != identity.claim
            || !valid_digest(&self.handoff.digest)
            || self.handoff.create_key != identity.payload.handoff.keys.create
            || self.handoff.invite_key != identity.payload.handoff.keys.invite
            || self.handoff.send_key != identity.payload.handoff.keys.send
            || match &identity.payload.handoff.channel {
                HandoffChannel::Existing { thread } => self.handoff.thread.as_ref() != Some(thread),
                HandoffChannel::New { .. } => self.handoff.thread.is_some(),
            }
        {
            return Err("bootstrap attachment mismatch");
        }
        Ok(())
    }
}
impl CompleteLinkedBootstrap {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.identity.validate()?;
        self.attachment.validate(&self.identity)?;
        self.retained.validate()?;
        if self.operation != self.identity.payload.linked_complete_key
            || self.legacy_completion.identity != self.attachment.handoff
            || self.legacy_completion.operation != self.identity.payload.handoff.keys.complete
            || self.retained.launch != self.identity.payload.launch
            || match &self.identity.payload.handoff.channel {
                HandoffChannel::Existing { thread } => {
                    self.attachment.handoff.thread.as_ref() != Some(thread)
                        || thread != &self.retained.thread
                }
                HandoffChannel::New { .. } => self.attachment.handoff.thread.is_some(),
            }
            || self.retained.recipient != self.attachment.resolved_seat
            || self.retained.pane != self.attachment.created.root_pane
            || self.retained.terminal != self.attachment.created.terminal
            || self.retained.host_incarnation != self.attachment.created.host_incarnation
        {
            return Err("linked bootstrap completion mismatch");
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod topology_contract_tests {
    use super::*;
    pub(crate) fn claim() -> CallerClaim {
        CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("sender"),
            binding_generation: 1,
            role: crate::protocol::authority::CallerRole::TopLevel,
            harness: crate::protocol::authority::Harness::Codex,
            native_session: crate::protocol::ids::NativeSessionId::new("session"),
            execution: crate::protocol::ids::ExecutionId::new("exec"),
            target: HostTargetId::new("w1:p1"),
        }
    }
    pub(crate) fn payload() -> BootstrapPayload {
        BootstrapPayload {
            handoff: HandoffPayload {
                namespace: HandoffNamespace {
                    instance: "i".into(),
                    state_dir: "/state".into(),
                    host_endpoint: "/host.sock".into(),
                },
                keys: HandoffKeys {
                    compound: OperationId::new("compound"),
                    begin: OperationId::new("begin"),
                    create: OperationId::new("create"),
                    invite: OperationId::new("invite"),
                    send: OperationId::new("send"),
                    complete: OperationId::new("complete"),
                },
                channel: HandoffChannel::Existing {
                    thread: ThreadId::new("thread"),
                },
                body: "work".into(),
            },
            workspace: HostTargetId::new("w1"),
            cwd: "/cwd".into(),
            label: "peer".into(),
            focus: false,
            env: Default::default(),
            launch: BootstrapLaunch {
                harness: crate::protocol::authority::Harness::Codex,
                binary: Some("/bin/codex".into()),
                name: Some("peer".into()),
                argv: vec!["--model".into(), "fixed".into()],
            },
            handoff_key: OperationId::new("child"),
            resolve_key: OperationId::new("resolve"),
            attach_key: OperationId::new("attach"),
            linked_complete_key: OperationId::new("linked-complete"),
        }
    }
    pub(crate) fn identity() -> BootstrapIdentity {
        let claim = claim();
        let mut identity = BootstrapIdentity {
            compound: payload().handoff.keys.compound.clone(),
            scope: crate::cli::journal::IntentScope::Cooperative {
                instance: claim.instance.clone(),
                seat: claim.seat.clone(),
            },
            claim,
            digest: "a".repeat(64),
            payload: payload(),
        };
        identity.digest = identity.semantic_digest().unwrap();
        identity
    }
    pub(crate) fn created() -> crate::ports::CreatedTab {
        use crate::host::continuity::{LocalEndpointWitness, SocketIdentity};
        crate::ports::CreatedTab {
            correlation: HostCallId::new("correlation"),
            workspace: HostTargetId::new("w1"),
            tab: HostTargetId::new("w1:t2"),
            root_pane: HostTargetId::new("w1:p2"),
            terminal: TerminalId::new("terminal"),
            host_incarnation: HostBootId::new("herdr-server:pid=42:start=1.000002:uid=501"),
            witness: LocalEndpointWitness {
                schema: 1,
                platform: "macos-proc-bsdinfo-v1".into(),
                endpoint: "/host.sock".into(),
                peer_uid: 501,
                peer_pid: 42,
                start_seconds: 1,
                start_microseconds: 2,
                socket: SocketIdentity {
                    device: 1,
                    inode: 2,
                    birth_seconds: 1,
                    birth_nanoseconds: 0,
                    change_seconds: 1,
                    change_nanoseconds: 0,
                },
            },
        }
    }
    pub(crate) fn attachment() -> BootstrapAttachment {
        BootstrapAttachment {
            attempt: BootstrapAttempt::first(),
            created: created(),
            resolve_operation: payload().resolve_key,
            resolved_seat: SeatId::new("peer"),
            handoff: HandoffIdentity {
                compound: OperationId::new("child"),
                digest: "b".repeat(64),
                claim: claim(),
                thread: Some(ThreadId::new("thread")),
                recipient: SeatId::new("peer"),
                create_key: OperationId::new("create"),
                invite_key: OperationId::new("invite"),
                send_key: OperationId::new("send"),
            },
        }
    }
    pub(crate) fn completion() -> CompleteLinkedBootstrap {
        use sha2::{Digest, Sha256};
        let report = serde_json::json!({"outcome":"started", "pane":"w1:p2", "seat":"peer", "harness":"codex", "argv":["--model","fixed"]});
        CompleteLinkedBootstrap {
            identity: identity(),
            attachment: attachment(),
            operation: payload().linked_complete_key,
            legacy_completion: HandoffMutation {
                identity: attachment().handoff,
                operation: payload().handoff.keys.complete,
            },
            retained: LinkedBootstrapReport {
                thread: ThreadId::new("thread"),
                recipient: SeatId::new("peer"),
                pane: HostTargetId::new("w1:p2"),
                kind: "launch".into(),
                launch: payload().launch,
                report_digest: format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&report).unwrap())
                ),
                report,
                terminal: created().terminal,
                host_incarnation: created().host_incarnation,
            },
        }
    }
    pub(crate) fn commands() -> Vec<crate::protocol::commands::Command> {
        use crate::protocol::commands::Command;
        let attempt = BootstrapAttempt::first();
        let mut recovery = RecoverBootstrap {
            inspection: None,
            identity: identity(),
            expected_attempt: attempt,
            operation: OperationId::new("decision"),
            disposition: BootstrapRecoveryDisposition::NotCreated {
                quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
            },
        };
        recovery.operation = recovery.decision_operation().unwrap();
        vec![
            Command::BeginBootstrap(Box::new(BeginBootstrap {
                identity: identity(),
                operation: payload().handoff.keys.begin,
            })),
            Command::ReserveBootstrapAttempt(Box::new(ReserveBootstrapAttempt {
                identity: identity(),
                expected_attempt: attempt,
                operation: attempt.operation(&identity().compound, "reserve").unwrap(),
            })),
            Command::RecordBootstrapNotSubmitted(Box::new(RecordBootstrapNotSubmitted {
                identity: identity(),
                expected_attempt: attempt,
                operation: attempt
                    .operation(&identity().compound, "not_submitted")
                    .unwrap(),
            })),
            Command::RecordBootstrapCreated(Box::new(RecordBootstrapCreated {
                identity: identity(),
                expected_attempt: attempt,
                operation: attempt.operation(&identity().compound, "record").unwrap(),
                evidence: created(),
            })),
            Command::AttachBootstrapHandoff(Box::new(AttachBootstrapHandoff {
                identity: identity(),
                operation: payload().attach_key,
                attachment: attachment(),
            })),
            Command::CompleteLinkedBootstrap(Box::new(completion())),
            Command::CheckBootstrapSubmission(Box::new(CheckBootstrapSubmission {
                identity: identity(),
                expected_attempt: attempt,
                operation: attempt.operation(&identity().compound, "check").unwrap(),
                expected_administrative_revision: 0,
            })),
            Command::BootstrapStatus(Box::new(BootstrapStatus {
                identity: identity(),
            })),
            Command::RecoverBootstrap(Box::new(recovery)),
        ]
    }
    #[test]
    fn topology_contract_structural_validation() {
        assert!(payload().validate().is_ok());
        let mut bad = payload();
        bad.focus = true;
        assert!(bad.validate().is_err(), "focus=true must not be admitted");
        let mut bad = payload();
        bad.env.insert("HERDR_PANE_ID".into(), "caller".into());
        assert!(bad.validate().is_err());
        let mut bad = payload();
        bad.cwd = "relative".into();
        assert!(bad.validate().is_err());
        let mut bad = payload();
        bad.handoff.namespace.state_dir = "/state/../other".into();
        assert!(bad.validate().is_err());
        let mut bad = payload();
        bad.handoff.keys.invite = bad.handoff.keys.send.clone();
        assert!(bad.validate().is_err());
        let mut bad = identity();
        bad.claim.role = crate::protocol::authority::CallerRole::Subagent;
        assert!(bad.validate().is_err());
        let mut bad = identity();
        bad.claim.instance = "foreign".into();
        assert!(bad.validate().is_err());
    }
    #[test]
    fn topology_contract_attempt_recovery_and_linked_report_shapes() {
        use sha2::Digest;
        assert!(BootstrapAttempt::new(0).is_err());
        assert!(serde_json::from_str::<BootstrapAttempt>("0").is_err());
        assert_ne!(
            BootstrapAttempt::first()
                .operation(&identity().compound, "reserve")
                .unwrap(),
            BootstrapAttempt::new(2)
                .unwrap()
                .operation(&identity().compound, "reserve")
                .unwrap()
        );
        let cancel = |reason: String| BootstrapRecoveryDisposition::Cancelled {
            reason,
            quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
            child_guard: BootstrapCancellationGuard {
                attached_child: None,
            },
        };
        assert!(
            cancel(" ".into()).validate().is_err(),
            "blank cancellation reason must refuse"
        );
        assert!(cancel("é".repeat(2048)).validate().is_ok());
        assert!(cancel("é".repeat(2049)).validate().is_err());
        assert!(cancel("x".repeat(4096)).validate().is_ok());
        assert!(cancel("x".repeat(4097)).validate().is_err());
        assert!(created().validate().is_ok());
        for platform in ["macos", "linux", "linux-proc-stat-v1", "unknown"] {
            let mut bad = created();
            bad.witness.platform = platform.into();
            assert!(
                bad.validate().is_err(),
                "unproduced platform admitted: {platform}"
            );
        }
        let mut bad = created();
        bad.witness.peer_pid = 0;
        assert!(bad.validate().is_err());
        let mut bad = created();
        bad.host_incarnation = HostBootId::new("foreign");
        assert!(bad.validate().is_err());
        assert!(
            serde_json::from_value::<crate::ports::CreatedTab>(serde_json::json!({"terminal":""}))
                .is_err()
        );
        assert!(completion().validate().is_ok());
        let mut bad = completion();
        bad.retained.thread = ThreadId::new("other-thread");
        assert!(bad.validate().is_err());
        let mut bad = completion();
        bad.retained.launch.argv.push("other".into());
        assert!(bad.validate().is_err());
        let mut bad = completion();
        bad.attachment.handoff.compound = OperationId::new("other-child");
        assert!(bad.validate().is_err());
        let mut bad = completion();
        bad.retained.kind = "delivery".into();
        assert!(bad.validate().is_err());
        let mut bad = completion();
        bad.retained.recipient = SeatId::new("other-peer");
        assert!(bad.validate().is_err());
        let mut bad = completion();
        bad.retained.report_digest = "a".repeat(64);
        assert!(bad.validate().is_err());
        for outcome in ["failed", "outcome_unknown"] {
            let mut bad = completion();
            bad.retained.report["outcome"] = outcome.into();
            bad.retained.report_digest = format!(
                "{:x}",
                sha2::Sha256::digest(serde_json::to_vec(&bad.retained.report).unwrap())
            );
            assert!(bad.validate().is_err());
        }
        let mut bad = completion();
        bad.retained.report = serde_json::Value::Null;
        assert!(bad.validate().is_err());
        let mut missing = serde_json::to_value(completion()).unwrap();
        missing.as_object_mut().unwrap().remove("retained");
        assert!(serde_json::from_value::<CompleteLinkedBootstrap>(missing).is_err());
        let mut recovery = match commands().pop().unwrap() {
            crate::protocol::commands::Command::RecoverBootstrap(v) => *v,
            _ => unreachable!(),
        };
        let original_key = recovery.operation.clone();
        recovery.expected_attempt = BootstrapAttempt::new(2).unwrap();
        assert_ne!(recovery.decision_operation().unwrap(), original_key);
        assert!(
            crate::protocol::commands::Command::RecoverBootstrap(Box::new(recovery))
                .validate()
                .is_err()
        );
        let mut missing = serde_json::to_value(commands().pop().unwrap()).unwrap();
        missing["args"]
            .as_object_mut()
            .unwrap()
            .remove("expected_attempt");
        assert!(serde_json::from_value::<crate::protocol::commands::Command>(missing).is_err());
        const LEGACY_CHILD: &str = r#"{"identity":{"compound":"child","digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","claim":{"instance":"i","seat":"sender","binding_generation":1,"role":"top_level","harness":"codex","native_session":"session","execution":"exec","target":"w1:p1"},"thread":"thread","recipient":"peer","create_key":"create","invite_key":"invite","send_key":"send"},"operation":"complete"}"#;
        assert_eq!(
            serde_json::to_string(&completion().legacy_completion).unwrap(),
            LEGACY_CHILD
        );
        assert_eq!(
            format!(
                "{:x}",
                sha2::Sha256::digest(
                    serde_json::to_vec(&serde_json::json!([
                        "complete_handoff",
                        completion().legacy_completion
                    ]))
                    .unwrap()
                )
            ),
            "c9722e3662c4af0d69f1da919f03984d14d8ae9bbddca3a69f63fea9ace15524"
        );
        assert_eq!(
            crate::store::control::cooperative_payload_hash(
                "complete_handoff",
                &completion().legacy_completion
            )
            .unwrap()
            .to_vec(),
            sha2::Sha256::digest(
                serde_json::to_vec(&serde_json::json!([
                    "complete_handoff",
                    completion().legacy_completion
                ]))
                .unwrap()
            )
            .to_vec()
        );
        let status = BootstrapResult {
            compound: identity().compound,
            attempt: BootstrapAttempt::first(),
            attempt_state: BootstrapAttemptState::Created,
            state: BootstrapState::Cancelled,
            creation: None,
            attachment: None,
            completed: None,
            recovery: None,
        };
        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value["state"], "cancelled");
        assert!(value.get("authorization").is_none());
        assert!(value.get("submission_authorization").is_none());
    }
    #[test]
    fn topology_not_submitted_wire_has_a_distinct_exact_attempt_key() {
        use crate::protocol::commands::Command;
        let raw = serde_json::json!({"kind":"record_bootstrap_not_submitted","args":{"identity":identity(),"expected_attempt":1,"operation":"bootstrap-7346d63b10526b3971fe0392dfb17c4ecafc7453a6383f2336ec0d67a89f1aa3"}});
        let command: Command = serde_json::from_value(raw)
            .expect("proven non-submission needs its own additive typed command");
        command.validate().unwrap();
    }
    #[test]
    fn recovery_inspection_wire_keys_bind_canonical_state_and_preserve_absent_history() {
        let status = BootstrapResult {
            compound: OperationId::new("b"),
            attempt: BootstrapAttempt::first(),
            attempt_state: BootstrapAttemptState::Prepared,
            state: BootstrapState::Prepared,
            creation: None,
            attachment: None,
            completed: None,
            recovery: None,
        };
        let inspection = BootstrapRecoveryInspection::from_status(&status).unwrap();
        // Independently SHA-256 of the literal domain-tagged canonical JSON tuple.
        assert_eq!(
            inspection.digest,
            "ea2b63590714039519cdc86b4d5cfe1accd9028e92d0542e2df5fdf301d16b76"
        );
        let mut request = match commands().pop().unwrap() {
            crate::protocol::commands::Command::RecoverBootstrap(v) => *v,
            _ => unreachable!(),
        };
        let old = serde_json::to_vec(&request).unwrap();
        let old_key = request.operation.clone();
        assert!(
            serde_json::from_slice::<serde_json::Value>(&old)
                .unwrap()
                .get("inspection")
                .is_none()
        );
        assert_eq!(
            serde_json::to_vec(&serde_json::from_slice::<RecoverBootstrap>(&old).unwrap()).unwrap(),
            old
        );
        request.inspection = Some(inspection);
        assert!(
            crate::protocol::commands::Command::RecoverBootstrap(Box::new(request.clone()))
                .validate()
                .is_err()
        );
        request.operation = request.decision_operation().unwrap();
        assert_ne!(request.operation, old_key);
        crate::protocol::commands::Command::RecoverBootstrap(Box::new(request.clone()))
            .validate()
            .unwrap();
        for bad in [
            BootstrapRecoveryInspection {
                version: 2,
                digest: "a".repeat(64),
            },
            BootstrapRecoveryInspection {
                version: 1,
                digest: "A".repeat(64),
            },
            BootstrapRecoveryInspection {
                version: 1,
                digest: "a".repeat(65),
            },
        ] {
            request.inspection = Some(bad);
            assert!(
                crate::protocol::commands::Command::RecoverBootstrap(Box::new(request.clone()))
                    .validate()
                    .is_err()
            );
        }
        for field in [
            "attempt",
            "possible",
            "unknown",
            "cancelled",
            "creation",
            "attachment",
            "recovery",
        ] {
            let mut changed = status.clone();
            match field {
                "attempt" => changed.attempt = BootstrapAttempt::new(2).unwrap(),
                "possible" => changed.attempt_state = BootstrapAttemptState::PossibleCreation,
                "unknown" => changed.attempt_state = BootstrapAttemptState::OutcomeUnknown,
                "cancelled" => changed.state = BootstrapState::Cancelled,
                "creation" => changed.creation = Some(created()),
                "attachment" => changed.attachment = Some(completion().attachment),
                "recovery" => {
                    changed.recovery = Some(Box::new(BootstrapRecoveryResult {
                        inspection: None,
                        identity: identity(),
                        attempt: BootstrapAttempt::first(),
                        operation: OperationId::new("old"),
                        disposition: BootstrapRecoveryDisposition::NotCreated {
                            quiescence:
                                BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
                        },
                        operator_uid: 501,
                        operator_provenance: "operator:local-user:501".into(),
                        creation: None,
                        state: BootstrapState::Prepared,
                    }))
                }
                _ => unreachable!(),
            }
            assert_ne!(
                BootstrapRecoveryInspection::from_status(&changed)
                    .unwrap()
                    .digest,
                "ea2b63590714039519cdc86b4d5cfe1accd9028e92d0542e2df5fdf301d16b76",
                "{field} omitted from inspected tuple"
            );
        }
        let mut oversized = status.clone();
        let mut huge = created();
        huge.witness.endpoint = ("/".to_owned() + &"x".repeat(512 * 1024 + 1024)).into();
        oversized.creation = Some(huge);
        assert!(BootstrapRecoveryInspection::from_status(&oversized).is_err());
    }
    #[test]
    fn inspection_aware_results_preserve_old_history_and_older_decoders_fail_closed() {
        // Shipped deny_unknown_fields field shape. Unchanged payload fields
        // use IgnoredAny: the input is already a genuine typed serialized result.
        // This tests old shape rejection, not a historical executable or malformed
        // old payload admission. Nested recovery still uses the old result shape.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct OldRequest {
            identity: serde::de::IgnoredAny,
            expected_attempt: serde::de::IgnoredAny,
            operation: serde::de::IgnoredAny,
            disposition: serde::de::IgnoredAny,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct OldRecovery {
            identity: serde::de::IgnoredAny,
            attempt: serde::de::IgnoredAny,
            operation: serde::de::IgnoredAny,
            disposition: serde::de::IgnoredAny,
            operator_uid: serde::de::IgnoredAny,
            operator_provenance: serde::de::IgnoredAny,
            creation: serde::de::IgnoredAny,
            state: serde::de::IgnoredAny,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct OldStatus {
            compound: serde::de::IgnoredAny,
            attempt: serde::de::IgnoredAny,
            attempt_state: serde::de::IgnoredAny,
            state: serde::de::IgnoredAny,
            creation: serde::de::IgnoredAny,
            attachment: serde::de::IgnoredAny,
            completed: serde::de::IgnoredAny,
            recovery: Option<Box<OldRecovery>>,
        }
        let mut result = BootstrapRecoveryResult {
            inspection: None,
            identity: identity(),
            attempt: BootstrapAttempt::first(),
            operation: OperationId::new("retained"),
            disposition: BootstrapRecoveryDisposition::NotCreated {
                quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
            },
            operator_uid: 501,
            operator_provenance: "operator:local-user:501".into(),
            creation: None,
            state: BootstrapState::Prepared,
        };
        let mut request = RecoverBootstrap {
            inspection: None,
            identity: result.identity.clone(),
            expected_attempt: result.attempt,
            operation: OperationId::new("temporary"),
            disposition: result.disposition.clone(),
        };
        request.operation = request.decision_operation().unwrap();
        let old_request = serde_json::to_vec(&request).unwrap();
        assert!(serde_json::from_slice::<OldRequest>(&old_request).is_ok());
        assert_eq!(
            serde_json::to_vec(&serde_json::from_slice::<RecoverBootstrap>(&old_request).unwrap())
                .unwrap(),
            old_request
        );
        request.inspection = Some(BootstrapRecoveryInspection {
            version: 1,
            digest: "a".repeat(64),
        });
        request.operation = request.decision_operation().unwrap();
        assert!(
            serde_json::from_value::<OldRequest>(serde_json::to_value(&request).unwrap()).is_err()
        );
        let old = serde_json::to_vec(&result).unwrap();
        assert!(serde_json::from_slice::<OldRecovery>(&old).is_ok());
        assert_eq!(
            serde_json::to_vec(&serde_json::from_slice::<BootstrapRecoveryResult>(&old).unwrap())
                .unwrap(),
            old
        );
        let mut status = BootstrapResult {
            compound: result.identity.compound.clone(),
            attempt: BootstrapAttempt::new(2).unwrap(),
            attempt_state: BootstrapAttemptState::Prepared,
            state: BootstrapState::Prepared,
            creation: None,
            attachment: None,
            completed: None,
            recovery: Some(Box::new(result.clone())),
        };
        assert!(
            serde_json::from_value::<OldStatus>(serde_json::to_value(&status).unwrap()).is_ok()
        );
        result.inspection = Some(BootstrapRecoveryInspection {
            version: 1,
            digest: "a".repeat(64),
        });
        assert!(
            serde_json::from_value::<OldRecovery>(serde_json::to_value(&result).unwrap()).is_err()
        );
        status.recovery = Some(Box::new(result));
        assert!(
            serde_json::from_value::<OldStatus>(serde_json::to_value(&status).unwrap()).is_err()
        );
        let decoded: BootstrapResult =
            serde_json::from_value(serde_json::to_value(&status).unwrap()).unwrap();
        assert!(
            decoded.recovery.unwrap().inspection.is_some(),
            "new presentation cannot strip guard for older decoders"
        );
    }
}
