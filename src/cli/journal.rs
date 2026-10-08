//! Durable, private CLI mutation intent.
use crate::protocol::{
    authority::CallerClaim,
    commands::*,
    ids::*,
    output::{OutputFormat, OutputSpec, encode_selected},
    pagination::{
        Consistency, Cursor, CursorDirection, CursorScope, Page, PageRequest, StopReason,
    },
    results::{ApiError, CommandResult, ErrorCode, IntentKind, IntentStatus, LocalIntent},
    time::UtcMillis,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IntentScope {
    Cooperative {
        instance: String,
        seat: SeatId,
    },
    Native {
        instance: String,
        seat: SeatId,
    },
    Operator {
        instance: String,
        local_user_uid: u32,
    },
    ServiceAllocation {
        instance: String,
        target: HostTargetId,
    },
    /// TRUST-POLICY C1: a seatless resume-only continuity check-in. There is
    /// no seat to scope to; the pane's target under one instance is the
    /// authority scope and the key the hook replays by.
    Continuity {
        instance: String,
        target: HostTargetId,
    },
}

/// Actor of the validated durable payload, independent of today's binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalActor {
    Agent,
    HumanOrOperator,
}

pub fn classify_original_actor(
    scope: &IntentScope,
    semantic: &SemanticMutation,
) -> io::Result<OriginalActor> {
    semantic.validate()?;
    if !scope_matches(scope, semantic) {
        return Err(invalid("intent authority scope mismatch"));
    }
    if let Some(claim) = semantic.frozen_claim() {
        return Ok(
            if claim.harness == crate::protocol::authority::Harness::Human {
                OriginalActor::HumanOrOperator
            } else {
                OriginalActor::Agent
            },
        );
    }
    match (scope, semantic) {
        (IntentScope::Operator { .. }, semantic) if semantic.is_operator() => {
            Ok(OriginalActor::HumanOrOperator)
        }
        (IntentScope::Native { .. }, SemanticMutation::Handoff(_)) => {
            Err(invalid("handoff needs frozen caller"))
        }
        (IntentScope::Native { .. }, _)
        | (IntentScope::Continuity { .. }, SemanticMutation::ContinuityCheckIn { .. })
        | (IntentScope::ServiceAllocation { .. }, SemanticMutation::ResolveSeat { .. }) => {
            Ok(OriginalActor::Agent)
        }
        _ => Err(invalid("unsupported original intent actor")),
    }
}

/// Native evidence is refreshed; cooperative claims are frozen as durable payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SemanticMutation {
    Handoff(Box<super::handoff::HandoffPlan>),
    Frozen {
        claim: CallerClaim,
        mutation: Box<SemanticMutation>,
    },
    CooperativeCheckIn {
        claim: CallerClaim,
        mode: CheckInMode,
        event_id: String,
        /// A person's explicit override of the agent-to-human guard
        /// (`me init --operator`); submitted as `Command::OperatorCheckIn`.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        operator: bool,
    },
    ResolveSeat {
        target: HostTargetId,
    },
    /// TRUST-POLICY C1: resume-only seatless reattachment request. `event_id`
    /// is the hook event that recorded it; `execution` is the successor
    /// binding's execution id, fixed once so every retry sends the same value.
    ContinuityCheckIn {
        target: HostTargetId,
        harness: crate::protocol::authority::Harness,
        native_session: NativeSessionId,
        source: String,
        event_id: String,
        execution: ExecutionId,
    },
    CheckIn,
    CreateThread {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        topic: String,
        goal: String,
    },
    Invite {
        thread: ThreadId,
        seat: SeatId,
        deadline_millis: Option<u64>,
    },
    Accept {
        thread: ThreadId,
    },
    Reject {
        thread: ThreadId,
        invitation: InvitationId,
        reason: String,
    },
    AcceptRequired {
        thread: ThreadId,
        invitation: InvitationId,
        requirement: RequirementId,
        expected_revision: u64,
    },
    SendMessage {
        #[serde(default, skip_serializing_if = "DeliveryMode::is_ordinary")]
        delivery_mode: DeliveryMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_intent: Option<crate::protocol::summary::UserIntent>,
        thread: ThreadId,
        body: String,
        invited_recipients: Vec<SeatId>,
        deadline_millis: Option<u64>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        relays_user: bool,
    },
    Ack {
        messages: Vec<MessageId>,
    },
    AckDisplayed {
        messages: Vec<MessageId>,
    },
    Leave {
        thread: ThreadId,
    },
    SetThreadName {
        thread: ThreadId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    SetTopic {
        thread: ThreadId,
        topic: String,
    },
    Archive {
        thread: ThreadId,
    },
    Reopen {
        thread: ThreadId,
    },
    OperatorRebind {
        seat: SeatId,
        target: HostTargetId,
    },
    OperatorFreshSeat {
        target: HostTargetId,
    },
    OperatorRetire {
        seat: SeatId,
    },
    OperatorReplace {
        seat: SeatId,
        target: HostTargetId,
        replace: SeatId,
    },
    OperatorOrphanInvite {
        thread: ThreadId,
        seat: SeatId,
        deadline_millis: Option<u64>,
    },
}
impl SemanticMutation {
    pub fn freeze(mutation: Self, claim: CallerClaim) -> io::Result<Self> {
        if mutation.is_operator()
            || matches!(
                mutation,
                Self::ResolveSeat { .. }
                    | Self::ContinuityCheckIn { .. }
                    | Self::CheckIn
                    | Self::Frozen { .. }
                    | Self::CooperativeCheckIn { .. }
            )
        {
            return Err(invalid("mutation cannot be frozen in cooperative context"));
        }
        let frozen = Self::Frozen {
            claim,
            mutation: Box::new(mutation),
        };
        frozen.validate()?;
        Ok(frozen)
    }
    pub fn is_lazy_send(&self) -> bool {
        match self {
            Self::Frozen { mutation, .. } => mutation.is_lazy_send(),
            Self::SendMessage {
                delivery_mode: DeliveryMode::Lazy,
                ..
            } => true,
            _ => false,
        }
    }
    pub fn frozen_claim(&self) -> Option<&CallerClaim> {
        match self {
            Self::Frozen { claim, .. } | Self::CooperativeCheckIn { claim, .. } => Some(claim),
            _ => None,
        }
    }

    /// Mirrors the deterministic checks in Command::validate without creating
    /// a placeholder caller claim. This runs before an ordinal is reserved.
    pub fn validate(&self) -> io::Result<()> {
        if let Some(claim) = self.frozen_claim()
            && (claim.instance.is_empty()
                || claim.role != crate::protocol::authority::CallerRole::TopLevel)
        {
            return Err(invalid("top-level cooperative context required"));
        }
        match self {
            Self::Handoff(plan) => plan.validate(),
            Self::Frozen { mutation, .. } => {
                if mutation.is_operator()
                    || matches!(
                        mutation.as_ref(),
                        Self::ResolveSeat { .. }
                            | Self::ContinuityCheckIn { .. }
                            | Self::CheckIn
                            | Self::Frozen { .. }
                            | Self::CooperativeCheckIn { .. }
                    )
                {
                    return Err(invalid("invalid nested cooperative mutation"));
                }
                mutation.validate()
            }
            Self::CooperativeCheckIn {
                claim,
                mode,
                event_id,
                operator,
            } => {
                if event_id.is_empty()
                    || event_id.len() > 1024
                    || event_id.chars().any(char::is_control)
                {
                    return Err(invalid("invalid lifecycle event identity"));
                }
                if *operator
                    && (claim.harness != crate::protocol::authority::Harness::Human
                        || !matches!(
                            mode,
                            crate::protocol::commands::CheckInMode::Lifecycle { .. }
                        ))
                {
                    return Err(invalid("operator check-in is a human lifecycle check-in"));
                }
                let check = CheckIn {
                    claim: claim.clone(),
                    mode: *mode,
                    operation: OperationId::new("validation"),
                };
                if *operator {
                    Command::OperatorCheckIn(check)
                } else {
                    Command::CheckIn(check)
                }
                .validate()
                .map_err(invalid)
            }
            Self::ContinuityCheckIn {
                target,
                harness,
                native_session,
                source,
                event_id,
                execution,
            } => {
                if event_id.is_empty()
                    || event_id.len() > 1024
                    || event_id.chars().any(char::is_control)
                {
                    return Err(invalid("invalid lifecycle event identity"));
                }
                ContinuityCheckIn {
                    target: target.clone(),
                    harness: *harness,
                    native_session: native_session.clone(),
                    source: source.clone(),
                    operation: OperationId::new("validation"),
                    execution: execution.clone(),
                }
                .validate()
                .map_err(invalid)
            }
            Self::Invite {
                deadline_millis: Some(0),
                ..
            }
            | Self::OperatorOrphanInvite {
                deadline_millis: Some(0),
                ..
            } => Err(invalid("deadline must be positive")),
            Self::CreateThread { goal, .. } if goal.is_empty() || goal.len() > 1024 => {
                Err(invalid("invalid thread goal byte bound"))
            }
            Self::CreateThread { name, .. } | Self::SetThreadName { name, .. } => name
                .as_deref()
                .map_or(Ok(()), |name| validate_thread_name(name).map_err(invalid)),
            Self::Reject { reason, .. } => validate_rejection_reason(reason).map_err(invalid),
            Self::AcceptRequired {
                expected_revision: 0,
                ..
            } => Err(invalid("required acceptance revision must be positive")),
            Self::Ack { messages } | Self::AckDisplayed { messages }
                if messages.is_empty() || messages.len() > MAX_BATCH_ITEMS =>
            {
                Err(invalid("invalid ack batch size"))
            }
            Self::SendMessage {
                delivery_mode: DeliveryMode::Lazy,
                invited_recipients,
                deadline_millis,
                ..
            } if !invited_recipients.is_empty() || deadline_millis.is_some() => Err(invalid(
                "lazy send cannot have explicit recipients or a deadline",
            )),
            Self::SendMessage {
                invited_recipients, ..
            } if invited_recipients.len() > MAX_BATCH_ITEMS => {
                Err(invalid("too many explicit recipients"))
            }
            _ => Ok(()),
        }
    }
    pub fn is_operator(&self) -> bool {
        matches!(
            self,
            Self::OperatorRebind { .. }
                | Self::OperatorFreshSeat { .. }
                | Self::OperatorRetire { .. }
                | Self::OperatorReplace { .. }
                | Self::OperatorOrphanInvite { .. }
        )
    }
    pub fn kind(&self) -> IntentKind {
        match self {
            Self::Handoff(_) => IntentKind::Handoff,
            Self::Frozen { mutation, .. } => mutation.kind(),
            Self::CooperativeCheckIn { .. } => IntentKind::CheckIn,
            Self::ResolveSeat { .. } => IntentKind::ResolveSeat,
            Self::ContinuityCheckIn { .. } => IntentKind::ContinuityCheckIn,
            Self::CheckIn => IntentKind::CheckIn,
            Self::CreateThread { .. } => IntentKind::CreateThread,
            Self::Invite { .. } => IntentKind::Invite,
            Self::Accept { .. } => IntentKind::Accept,
            Self::Reject { .. } => IntentKind::Reject,
            Self::AcceptRequired { .. } => IntentKind::Accept,
            Self::SendMessage { .. } => IntentKind::SendMessage,
            Self::Ack { .. } | Self::AckDisplayed { .. } => IntentKind::Ack,
            Self::Leave { .. } => IntentKind::Leave,
            Self::SetTopic { .. } => IntentKind::SetTopic,
            Self::SetThreadName { .. } => IntentKind::SetThreadName,
            Self::Archive { .. } => IntentKind::Archive,
            Self::Reopen { .. } => IntentKind::Reopen,
            Self::OperatorRebind { .. } => IntentKind::OperatorRebind,
            Self::OperatorFreshSeat { .. } => IntentKind::OperatorFreshSeat,
            Self::OperatorRetire { .. } => IntentKind::OperatorRetire,
            Self::OperatorReplace { .. } => IntentKind::OperatorReplace,
            Self::OperatorOrphanInvite { .. } => IntentKind::OperatorOrphanInvite,
        }
    }
    pub fn thread(&self) -> Option<&ThreadId> {
        match self {
            Self::Handoff(plan) => plan.request.thread.as_ref(),
            Self::Frozen { mutation, .. } => mutation.thread(),
            Self::Invite { thread, .. }
            | Self::Accept { thread }
            | Self::AcceptRequired { thread, .. }
            | Self::Reject { thread, .. }
            | Self::SendMessage { thread, .. }
            | Self::Leave { thread }
            | Self::SetTopic { thread, .. }
            | Self::SetThreadName { thread, .. }
            | Self::Archive { thread }
            | Self::Reopen { thread }
            | Self::OperatorOrphanInvite { thread, .. } => Some(thread),
            _ => None,
        }
    }
    pub fn to_command(
        &self,
        operation: OperationId,
        claim: Option<CallerClaim>,
    ) -> io::Result<Command> {
        let native = || {
            claim
                .clone()
                .ok_or_else(|| invalid("fresh caller claim required"))
        };
        let command = match self {
            Self::Handoff(_) => return Err(invalid("handoff requires compound coordinator")),
            Self::Frozen { claim, mutation } => {
                return mutation.to_command(operation, Some(claim.clone()));
            }
            Self::CooperativeCheckIn {
                claim,
                mode,
                operator,
                ..
            } => {
                let check = CheckIn {
                    claim: claim.clone(),
                    mode: *mode,
                    operation,
                };
                if *operator {
                    Command::OperatorCheckIn(check)
                } else {
                    Command::CheckIn(check)
                }
            }
            Self::ResolveSeat { target } => Command::ResolveSeat(ResolveSeat {
                target: target.clone(),
                operation,
            }),
            Self::ContinuityCheckIn {
                target,
                harness,
                native_session,
                source,
                execution,
                ..
            } => Command::ContinuityCheckIn(ContinuityCheckIn {
                target: target.clone(),
                harness: *harness,
                native_session: native_session.clone(),
                source: source.clone(),
                operation,
                execution: execution.clone(),
            }),
            Self::CheckIn => Command::CheckIn(CheckIn {
                mode: crate::protocol::commands::CheckInMode::Current,
                operation,
                claim: native()?,
            }),
            Self::CreateThread { name, topic, goal } => Command::CreateThread(CreateThread {
                name: name.clone(),
                topic: topic.clone(),
                goal: goal.clone(),
                operation,
                claim: native()?,
            }),
            Self::Invite {
                thread,
                seat,
                deadline_millis,
            } => Command::Invite(Invite {
                thread: thread.clone(),
                seat: seat.clone(),
                deadline_millis: *deadline_millis,
                operation,
                claim: native()?,
            }),
            Self::Reject {
                thread,
                invitation,
                reason,
            } => Command::Reject(Reject {
                thread: thread.clone(),
                invitation: invitation.clone(),
                reason: reason.clone(),
                operation,
                claim: native()?,
            }),
            Self::Accept { thread } => Command::Accept(Accept {
                thread: thread.clone(),
                operation,
                claim: native()?,
            }),
            Self::AcceptRequired {
                thread,
                invitation,
                requirement,
                expected_revision,
            } => Command::AcceptRequired(AcceptRequired {
                thread: thread.clone(),
                invitation: invitation.clone(),
                requirement: requirement.clone(),
                expected_revision: *expected_revision,
                operation,
                claim: native()?,
            }),
            Self::SendMessage {
                delivery_mode,
                thread,
                body,
                invited_recipients,
                deadline_millis,
                relays_user,
                user_intent,
            } => Command::SendMessage(SendMessage {
                delivery_mode: *delivery_mode,
                thread: thread.clone(),
                body: body.clone(),
                invited_recipients: invited_recipients.clone(),
                deadline_millis: *deadline_millis,
                operation,
                claim: native()?,
                relays_user: *relays_user,
                user_intent: *user_intent,
            }),
            Self::Ack { messages } => Command::Ack(Ack {
                messages: messages.clone(),
                operation,
                claim: native()?,
            }),
            Self::AckDisplayed { messages } => Command::AckDisplayed(Ack {
                messages: messages.clone(),
                operation,
                claim: native()?,
            }),
            Self::Leave { thread } => Command::Leave(Leave {
                thread: thread.clone(),
                operation,
                claim: native()?,
            }),
            Self::SetThreadName { thread, name } => Command::SetThreadName(SetThreadName {
                thread: thread.clone(),
                name: name.clone(),
                operation,
                claim: native()?,
            }),
            Self::SetTopic { thread, topic } => Command::SetTopic(SetTopic {
                thread: thread.clone(),
                topic: topic.clone(),
                operation,
                claim: native()?,
            }),
            Self::Archive { thread } => Command::Archive(ThreadMutation {
                thread: thread.clone(),
                operation,
                claim: native()?,
            }),
            Self::Reopen { thread } => Command::Reopen(ThreadMutation {
                thread: thread.clone(),
                operation,
                claim: native()?,
            }),
            Self::OperatorRebind { seat, target } => Command::OperatorRebind(OperatorRebind {
                seat: seat.clone(),
                target: target.clone(),
                operation,
            }),
            Self::OperatorRetire { seat } => {
                Command::OperatorRetire(crate::protocol::commands::OperatorRetire {
                    seat: seat.clone(),
                    operation,
                })
            }
            Self::OperatorReplace {
                seat,
                target,
                replace,
            } => Command::OperatorReplace(crate::protocol::commands::OperatorReplace {
                seat: seat.clone(),
                target: target.clone(),
                replace: replace.clone(),
                operation,
            }),
            Self::OperatorFreshSeat { target } => Command::OperatorFreshSeat(OperatorFreshSeat {
                target: target.clone(),
                operation,
            }),
            Self::OperatorOrphanInvite {
                thread,
                seat,
                deadline_millis,
            } => Command::OperatorOrphanInvite(OperatorOrphanInvite {
                thread: thread.clone(),
                seat: seat.clone(),
                deadline_millis: *deadline_millis,
                operation,
            }),
        };
        command.validate().map_err(invalid)?;
        Ok(command)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentRef {
    pub ordinal: u64,
    pub operation: OperationId,
}
impl IntentRef {
    pub fn recovery_ref(&self) -> String {
        format!("local:{}", self.ordinal)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentHeader {
    pub reference: IntentRef,
    pub scope: IntentScope,
    pub created_at_millis: i64,
    pub kind: IntentKind,
    pub thread: Option<ThreadId>,
    pub semantic_digest: String,
}
#[derive(Debug, Clone)]
pub struct PendingIntent {
    pub header: IntentHeader,
    pub semantic: SemanticMutation,
    pub operation: OperationId,
}
pub type PendingPage = Page<LocalIntent>;
pub struct Journal {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DisplayedProgress {
    instance: String,
    seat: SeatId,
    binding_generation: u64,
    execution: ExecutionId,
    message: MessageId,
    body_len: u64,
    flushed_through: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LazyDisplayedProgress {
    claim: CallerClaim,
    message: MessageId,
    body_len: u64,
    flushed_through: u64,
}
impl Journal {
    fn lazy_display_path(&self, claim: &CallerClaim, message: &MessageId) -> io::Result<PathBuf> {
        let mut digest = Sha256::new();
        digest.update(serde_json::to_vec(&(claim, message))?);
        Ok(self
            .root
            .join(format!("display-lazy-{:x}.progress", digest.finalize())))
    }
    /// Cooperative display hint only; no receipt or canonical delivery mutation.
    /// Full caller identity isolates progress across every occupant transition.
    pub fn record_lazy_displayed_chunk(
        &self,
        claim: &CallerClaim,
        message: &MessageId,
        start: u64,
        end: u64,
        body_len: u64,
    ) -> io::Result<bool> {
        if start > end || end > body_len || (start == end && body_len != 0) {
            return Err(invalid("invalid lazy inbox body span"));
        }
        let _lock = self.lock()?;
        let path = self.lazy_display_path(claim, message)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let prior: Option<LazyDisplayedProgress> = match options.open(&path) {
            Ok(file) => {
                let meta = file.metadata()?;
                if !meta.is_file() || meta.len() > 16384 {
                    return Err(invalid("unsafe lazy display progress"));
                }
                #[cfg(unix)]
                if meta.permissions().mode() & 0o077 != 0 {
                    return Err(invalid("lazy display progress is not private"));
                }
                let mut bytes = Vec::new();
                file.take(16385).read_to_end(&mut bytes)?;
                Some(
                    serde_json::from_slice(&bytes)
                        .map_err(|_| invalid("lazy display progress corrupt"))?,
                )
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if prior.as_ref().is_some_and(|p| {
            p.claim != *claim
                || p.message != *message
                || p.body_len != body_len
                || p.flushed_through > body_len
        }) {
            return Err(invalid("lazy display progress identity changed"));
        }
        let flushed = match &prior {
            _ if start == 0 => prior.as_ref().map_or(end, |p| p.flushed_through.max(end)),
            Some(p) if p.flushed_through == start => end,
            Some(p) if p.flushed_through >= end => p.flushed_through,
            _ => return Ok(false),
        };
        let progress = LazyDisplayedProgress {
            claim: claim.clone(),
            message: message.clone(),
            body_len,
            flushed_through: flushed,
        };
        let temp = self
            .root
            .join(format!(".display-lazy-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = private_new(&temp)?;
            serde_json::to_writer(&mut file, &progress)?;
            file.sync_all()?;
            fs::rename(&temp, &path)?;
            File::open(&self.root)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result?;
        Ok(flushed == body_len)
    }
    /// Clear only the frozen occupant's successfully settled lazy body proof.
    pub fn clear_lazy_displayed_chunk(
        &self,
        claim: &CallerClaim,
        message: &MessageId,
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        match fs::remove_file(self.lazy_display_path(claim, message)?) {
            Ok(()) => File::open(&self.root)?.sync_all(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref();
        if !root.exists() {
            #[cfg(unix)]
            {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(root)?;
            }
            #[cfg(not(unix))]
            {
                fs::create_dir_all(root)?;
            }
        }
        if !root.is_dir() || root.symlink_metadata()?.file_type().is_symlink() {
            return Err(invalid("unsafe intent directory"));
        }
        #[cfg(unix)]
        if root.metadata()?.permissions().mode() & 0o077 != 0 {
            return Err(invalid("intent directory is not private"));
        }
        let journal = Self {
            root: root.canonicalize()?,
        };
        let _lock = journal.lock()?;
        let marker = journal.root.join("journal-format");
        if marker.exists() {
            if fs::read_to_string(&marker)? != "1\n" {
                return Err(invalid("intent journal format corrupt"));
            }
            journal.counter()?;
        } else {
            if fs::read_dir(&journal.root)?.any(|entry| {
                entry
                    .ok()
                    .is_some_and(|e| e.file_name().to_string_lossy().ends_with(".intent"))
            }) {
                return Err(invalid("intent journal marker missing"));
            }
            if journal.root.join("next-ordinal").exists() {
                if journal.counter()? != 0 {
                    return Err(invalid("intent journal marker missing"));
                }
            } else {
                journal.reserve(0)?;
            }
            let mut file = private_new(&marker)?;
            file.write_all(b"1\n")?;
            file.sync_all()?;
            File::open(&journal.root)?.sync_all()?;
        }
        Ok(journal)
    }
    /// Validate an existing journal without allocating, locking or publishing metadata.
    pub fn read_only(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref();
        let metadata = root.symlink_metadata()?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(invalid("unsafe intent directory"));
        }
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid("intent directory is not private"));
        }
        let journal = Self {
            root: root.canonicalize()?,
        };
        if fs::read_to_string(journal.root.join("journal-format"))? != "1\n" {
            return Err(invalid("intent journal format corrupt"));
        }
        journal.counter()?;
        Ok(journal)
    }

    fn lock(&self) -> io::Result<File> {
        let file = private_open(&self.root.join("allocator.lock"))?;
        let start = std::time::Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) => {
                    if start.elapsed() >= std::time::Duration::from_secs(1) {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "intent allocator lock timeout",
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error),
            }
        }
    }
    fn counter(&self) -> io::Result<u64> {
        let path = self.root.join("next-ordinal");
        let mut highest_published = 0;
        for entry in fs::read_dir(&self.root)? {
            let name = entry?.file_name().to_string_lossy().to_string();
            if name.ends_with(".intent") {
                let ordinal: u64 = name
                    .get(..20)
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| invalid("invalid intent filename"))?;
                highest_published = highest_published.max(ordinal);
            }
        }
        if !path.exists() {
            return Err(invalid("intent allocator missing"));
        }
        let raw = fs::read_to_string(path)?;
        let counter: u64 = raw
            .trim_end_matches('\n')
            .parse()
            .map_err(|_| invalid("intent allocator corrupt"))?;
        if counter < highest_published {
            return Err(invalid("intent allocator behind published entries"));
        }
        Ok(counter)
    }
    fn reserve(&self, ordinal: u64) -> io::Result<()> {
        let path = self.root.join(format!(".counter-{}", Uuid::new_v4()));
        let mut file = private_new(&path)?;
        writeln!(file, "{ordinal}")?;
        file.sync_all()?;
        fs::rename(path, self.root.join("next-ordinal"))?;
        File::open(&self.root)?.sync_all()
    }
    fn path(&self, reference: &IntentRef) -> PathBuf {
        self.root.join(format!(
            "{:020}-{}.intent",
            reference.ordinal,
            reference.operation.as_str()
        ))
    }
    fn displayed_progress_path(&self, claim: &CallerClaim, message: &MessageId) -> PathBuf {
        let mut digest = Sha256::new();
        digest.update((claim.instance.len() as u64).to_be_bytes());
        digest.update(claim.instance.as_bytes());
        digest.update((claim.seat.as_str().len() as u64).to_be_bytes());
        digest.update(claim.seat.as_str().as_bytes());
        digest.update(claim.binding_generation.to_be_bytes());
        digest.update((claim.execution.as_str().len() as u64).to_be_bytes());
        digest.update(claim.execution.as_str().as_bytes());
        digest.update(message.as_str().as_bytes());
        self.root
            .join(format!("display-{:x}.progress", digest.finalize()))
    }

    fn read_displayed_progress(&self, path: &Path) -> io::Result<Option<DisplayedProgress>> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = match options.open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > 4096 {
            return Err(invalid("unsafe inbox display progress"));
        }
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid("inbox display progress is not private"));
        }
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(invalid("inbox display progress too large"));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| invalid("inbox display progress corrupt"))
    }

    /// Record a body span only after the selected inbox page was fully written
    /// and flushed. Client-local progress can only narrow automatic ACKs; the
    /// daemon still decides receipt eligibility from canonical state.
    pub fn record_displayed_chunk(
        &self,
        claim: &CallerClaim,
        message: &MessageId,
        start: u64,
        end: u64,
        body_len: u64,
    ) -> io::Result<bool> {
        if start > end || end > body_len || (start == end && body_len != 0) {
            return Err(invalid("invalid inbox body span"));
        }
        if start == 0 && end == body_len {
            return Ok(true);
        }
        let _lock = self.lock()?;
        let path = self.displayed_progress_path(claim, message);
        let prior = self.read_displayed_progress(&path)?;
        if prior.as_ref().is_some_and(|prior| {
            prior.instance != claim.instance
                || prior.seat != claim.seat
                || prior.binding_generation != claim.binding_generation
                || prior.execution != claim.execution
                || prior.message != *message
                || prior.body_len != body_len
        }) {
            return Err(invalid("inbox display progress identity changed"));
        }
        let flushed = match prior.as_ref() {
            _ if start == 0 => prior
                .as_ref()
                .map_or(end, |prior| prior.flushed_through.max(end)),
            Some(prior) if prior.flushed_through == start => end,
            Some(prior) if prior.flushed_through >= end => prior.flushed_through,
            _ => return Ok(false),
        };
        let progress = DisplayedProgress {
            instance: claim.instance.clone(),
            seat: claim.seat.clone(),
            binding_generation: claim.binding_generation,
            execution: claim.execution.clone(),
            message: message.clone(),
            body_len,
            flushed_through: flushed,
        };
        let temp = self.root.join(format!(".display-{}.tmp", Uuid::new_v4()));
        let mut file = private_new(&temp)?;
        serde_json::to_writer(&mut file, &progress)?;
        file.sync_all()?;
        fs::rename(&temp, &path)?;
        File::open(&self.root)?.sync_all()?;
        Ok(flushed == body_len)
    }

    pub fn clear_displayed_chunk(
        &self,
        claim: &CallerClaim,
        message: &MessageId,
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        let path = self.displayed_progress_path(claim, message);
        match fs::remove_file(path) {
            Ok(()) => File::open(&self.root)?.sync_all(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
    pub fn resolve_recovery_ref(&self, reference: &str) -> io::Result<IntentRef> {
        LocalRecoveryRef::parse(reference).map_err(invalid)?;
        let ordinal: u64 = reference
            .strip_prefix("local:")
            .unwrap()
            .parse()
            .map_err(invalid)?;
        if ordinal == 0 {
            return Err(invalid("invalid intent reference"));
        }
        if reference != format!("local:{ordinal}") {
            return Err(invalid("noncanonical intent reference"));
        }
        let prefix = format!("{ordinal:020}-");
        for entry in fs::read_dir(&self.root)? {
            let name = entry?.file_name().to_string_lossy().to_string();
            if let Some(operation) = name
                .strip_prefix(&prefix)
                .and_then(|s| s.strip_suffix(".intent"))
            {
                let operation = OperationId::parse(operation).map_err(invalid)?;
                if Uuid::parse_str(operation.as_str()).is_err() {
                    return Err(invalid("invalid intent operation"));
                }
                let result = IntentRef { ordinal, operation };
                self.load(&result)?;
                return Ok(result);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "intent reference not found",
        ))
    }
    pub fn record(
        &self,
        scope: IntentScope,
        semantic: SemanticMutation,
        created_at_millis: i64,
    ) -> io::Result<IntentRef> {
        if !scope_matches(&scope, &semantic) {
            return Err(invalid("intent authority scope mismatch"));
        }
        semantic.validate()?;
        let body = serde_json::to_vec(&semantic)?;
        let _lock = self.lock()?;
        self.record_locked(scope, semantic, created_at_millis, body)
    }
    /// Serialize event allocation and recover an intent published before context save.
    /// Lock order is context journal then CLI allocator; factory performs local work only.
    pub fn record_check_in(
        &self,
        scope: IntentScope,
        event_id: &str,
        created_at_millis: i64,
        factory: impl FnOnce() -> io::Result<(CallerClaim, CheckInMode)>,
    ) -> io::Result<IntentRef> {
        self.record_check_in_as(scope, event_id, created_at_millis, false, factory)
    }
    /// `record_check_in`, optionally flagged as the operator override. The scope scan
    /// reads headers through the hardened reader (see `pending_continuity`).
    pub fn record_check_in_as(
        &self,
        scope: IntentScope,
        event_id: &str,
        created_at_millis: i64,
        operator: bool,
        factory: impl FnOnce() -> io::Result<(CallerClaim, CheckInMode)>,
    ) -> io::Result<IntentRef> {
        let _lock = self.lock()?;
        for entry in fs::read_dir(&self.root)? {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("intent") {
                continue;
            }
            let Some(header) = read_intent_header(&path) else {
                continue;
            };
            if header.scope != scope {
                continue;
            }
            let pending = self.load(&header.reference)?;
            if matches!(&pending.semantic, SemanticMutation::CooperativeCheckIn { event_id: stored, .. } if stored == event_id)
            {
                return Ok(header.reference);
            }
        }
        let (claim, mode) = factory()?;
        let semantic = SemanticMutation::CooperativeCheckIn {
            claim,
            mode,
            event_id: event_id.into(),
            operator,
        };
        if !scope_matches(&scope, &semantic) {
            return Err(invalid("intent authority scope mismatch"));
        }
        semantic.validate()?;
        let body = serde_json::to_vec(&semantic)?;
        self.record_locked(scope, semantic, created_at_millis, body)
    }
    /// Look up an already published CheckIn without allocating a new key. The scan
    /// reads headers through the hardened reader (see `pending_continuity`).
    pub fn find_check_in(
        &self,
        scope: &IntentScope,
        event_id: &str,
    ) -> io::Result<Option<PendingIntent>> {
        let _lock = self.lock()?;
        for entry in fs::read_dir(&self.root)? {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("intent") {
                continue;
            }
            let Some(header) = read_intent_header(&path) else {
                continue;
            };
            if &header.scope != scope {
                continue;
            }
            let pending = self.load(&header.reference)?;
            if matches!(&pending.semantic, SemanticMutation::CooperativeCheckIn { event_id: stored, .. } if stored == event_id)
            {
                return Ok(Some(pending));
            }
        }
        Ok(None)
    }
    /// The pending continuity intent of this pane under `instance`, if a
    /// previous hook recorded one and did not finish it (lost reply, crash,
    /// retry window elapsed). Oldest first; it carries the operation key the
    /// daemon replays by. Called only on the resume path. The journal
    /// directory is shared with entries this scan does not own, so every read
    /// is hardened: `O_NOFOLLOW|O_NONBLOCK`, regular files only, a capped
    /// header read, and a vanished, unreadable or unparsable entry is skipped
    /// rather than failing the scan.
    pub fn pending_continuity(
        &self,
        instance: &str,
        target: &HostTargetId,
    ) -> io::Result<Option<PendingIntent>> {
        let scope = IntentScope::Continuity {
            instance: instance.into(),
            target: target.clone(),
        };
        let _lock = self.lock()?;
        let mut found: Vec<IntentRef> = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("intent") {
                continue;
            }
            let Some(header) = read_intent_header(&path) else {
                continue;
            };
            if header.scope == scope {
                found.push(header.reference);
            }
        }
        found.sort_by_key(|reference| reference.ordinal);
        Ok(found
            .into_iter()
            .find_map(|reference| self.load(&reference).ok()))
    }
    fn record_locked(
        &self,
        scope: IntentScope,
        semantic: SemanticMutation,
        created_at_millis: i64,
        body: Vec<u8>,
    ) -> io::Result<IntentRef> {
        let ordinal = self
            .counter()?
            .checked_add(1)
            .ok_or_else(|| invalid("intent allocator exhausted"))?;
        self.reserve(ordinal)?;
        let reference = IntentRef {
            ordinal,
            operation: OperationId::new(Uuid::new_v4().to_string()),
        };
        let header = IntentHeader {
            reference: reference.clone(),
            scope,
            created_at_millis,
            kind: semantic.kind(),
            thread: semantic.thread().cloned(),
            semantic_digest: format!("{:x}", Sha256::digest(&body)),
        };
        let path = self.root.join(format!(".intent-{}", Uuid::new_v4()));
        let mut file = private_new(&path)?;
        serde_json::to_writer(&mut file, &header)?;
        file.write_all(b"\n")?;
        file.write_all(&body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(path, self.path(&reference))?;
        File::open(&self.root)?.sync_all()?;
        Ok(reference)
    }
    pub fn load(&self, reference: &IntentRef) -> io::Result<PendingIntent> {
        let mut reader = BufReader::new(File::open(self.path(reference))?);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let header: IntentHeader = serde_json::from_str(&line)?;
        if &header.reference != reference {
            return Err(invalid("intent header mismatch"));
        }
        let mut body = Vec::new();
        reader.read_to_end(&mut body)?;
        let semantic: SemanticMutation = serde_json::from_slice(&body)?;
        semantic.validate()?;
        if format!("{:x}", Sha256::digest(serde_json::to_vec(&semantic)?)) != header.semantic_digest
            || semantic.kind() != header.kind
            || semantic.thread() != header.thread.as_ref()
            || !scope_matches(&header.scope, &semantic)
        {
            return Err(invalid("intent semantic mismatch"));
        }
        Ok(PendingIntent {
            operation: reference.operation.clone(),
            header,
            semantic,
        })
    }
    /// Idempotent hook completion after output flush, under the allocator lock.
    /// The scan reads headers through the hardened reader (see `pending_continuity`).
    pub fn complete_operation(&self, operation: &OperationId) -> io::Result<()> {
        let _lock = self.lock()?;
        for entry in fs::read_dir(&self.root)? {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("intent") {
                continue;
            }
            let Some(header) = read_intent_header(&path) else {
                continue;
            };
            if &header.reference.operation == operation {
                self.load(&header.reference)?;
                return self.complete(&header.reference);
            }
        }
        Ok(())
    }
    pub fn complete(&self, reference: &IntentRef) -> io::Result<()> {
        fs::remove_file(self.path(reference))?;
        File::open(&self.root)?.sync_all()
    }
    pub fn page(&self, request: &PageRequest) -> io::Result<PendingPage> {
        self.page_with_argv(request, &["pending-ops".to_owned()])
    }
    pub fn page_with_argv(
        &self,
        request: &PageRequest,
        prefix: &[String],
    ) -> io::Result<PendingPage> {
        self.page_with_output(
            request,
            prefix,
            &OutputSpec {
                format: OutputFormat::Text,
                ..OutputSpec::default()
            },
        )
        .map_err(api_io)
    }
    /// `prefix` is the validated command through `pending-ops`, with no page flags.
    pub fn page_with_output(
        &self,
        request: &PageRequest,
        prefix: &[String],
        output: &OutputSpec,
    ) -> Result<PendingPage, ApiError> {
        // Validate the finite page bounds here; cursor decoding below has its
        // own error code and must also run for an empty journal.
        PageRequest {
            cursor: None,
            limit: request.limit,
            max_bytes: request.max_bytes,
        }
        .validate()
        .map_err(|e| api(ErrorCode::InvalidRequest, e))?;
        output
            .validate()
            .map_err(|e| api(ErrorCode::InvalidRequest, e))?;
        validate_prefix(prefix, output)?;
        let identity =
            serde_json::to_vec(&(self.root.to_string_lossy().to_string(), prefix, output))
                .map_err(io_api)?;
        let key = format!("{:x}", Sha256::digest(identity));
        let (after, high) = if let Some(raw) = &request.cursor {
            let cursor = Cursor::decode(raw).map_err(|e| api(ErrorCode::InvalidCursor, e))?;
            if cursor
                .validate_for(
                    &key,
                    CursorScope::LocalIntents,
                    &key,
                    &key,
                    CursorDirection::Ascending,
                    1,
                )
                .is_err()
                || cursor.search.is_some()
                || cursor.last_examined_key.is_some()
                || cursor.after_ordinal >= cursor.high_water_ordinal
            {
                return Err(api(
                    ErrorCode::InvalidCursor,
                    "intent cursor scope mismatch",
                ));
            }
            (cursor.after_ordinal, cursor.high_water_ordinal)
        } else {
            let _lock = self.lock().map_err(io_api)?;
            (0, self.counter().map_err(io_api)?)
        };
        let mut nearest = BTreeMap::new();
        let mut overflow = false;
        let keep = usize::from(request.limit) + 1;
        for entry in fs::read_dir(&self.root).map_err(io_api)? {
            let entry = entry.map_err(io_api)?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".intent") {
                continue;
            }
            let ordinal: u64 = name
                .get(..20)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| api(ErrorCode::StoreCorrupt, "invalid intent filename"))?;
            if ordinal > after && ordinal <= high {
                nearest.insert(ordinal, entry.path());
                if nearest.len() > keep {
                    nearest.pop_last();
                    overflow = true;
                }
            }
        }
        let paths: Vec<_> = nearest.into_iter().collect();
        let mut page = PendingPage {
            items: Vec::new(),
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: high,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        };
        for (index, (ordinal, path)) in paths.iter().enumerate() {
            let file = match File::open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_api(error)),
            };
            let mut line = String::new();
            BufReader::with_capacity(1, file)
                .read_line(&mut line)
                .map_err(io_api)?;
            let header: IntentHeader = serde_json::from_str(&line).map_err(io_api)?;
            if header.reference.ordinal != *ordinal {
                return Err(api(ErrorCode::StoreCorrupt, "intent ordinal mismatch"));
            }
            let item = LocalIntent {
                operation: header.reference.operation.clone(),
                recovery_ref: LocalRecoveryRef::parse(header.reference.recovery_ref())
                    .map_err(|e| api(ErrorCode::StoreCorrupt, e))?,
                created_at: UtcMillis(header.created_at_millis),
                kind: header.kind,
                thread: header.thread,
                status: IntentStatus::Pending,
            };
            let mut candidate = page.clone();
            candidate.items.push(item);
            candidate.next_cursor = None;
            candidate.next_argv = None;
            candidate.has_more = false;
            candidate.stop_reason = StopReason::Complete;
            let more = index + 1 < paths.len() || overflow;
            if more {
                set_cursor(
                    &mut candidate,
                    &key,
                    request,
                    prefix,
                    *ordinal,
                    StopReason::Rows,
                )?;
            }
            let size = selected_len(&candidate, output)?;
            if candidate.items.len() > request.limit as usize || size > request.max_bytes as usize {
                if page.items.is_empty() {
                    return Err(budget_error(size));
                }
                let previous = page
                    .items
                    .last()
                    .unwrap()
                    .recovery_ref
                    .as_str()
                    .strip_prefix("local:")
                    .unwrap()
                    .parse()
                    .map_err(|_| api(ErrorCode::StoreCorrupt, "invalid local ordinal"))?;
                set_cursor(
                    &mut page,
                    &key,
                    request,
                    prefix,
                    previous,
                    if candidate.items.len() > request.limit as usize {
                        StopReason::Rows
                    } else {
                        StopReason::Bytes
                    },
                )?;
                let final_size = selected_len(&page, output)?;
                if final_size > request.max_bytes as usize {
                    return Err(budget_error(final_size));
                }
                break;
            }
            page = candidate;
        }
        if page.items.is_empty() && overflow {
            let mut error = api(
                ErrorCode::ReadBudgetExhausted,
                "intent page changed during traversal; retry same cursor",
            );
            let mut argv = prefix.to_vec();
            if let Some(cursor) = &request.cursor {
                argv.extend(["--cursor".into(), cursor.clone()]);
            }
            argv.extend([
                "--limit".into(),
                request.limit.to_string(),
                "--max-bytes".into(),
                request.max_bytes.to_string(),
            ]);
            error.restart_argv = Some(argv);
            return Err(error);
        }
        let final_size = selected_len(&page, output)?;
        if final_size > request.max_bytes as usize {
            return Err(budget_error(final_size));
        }
        page.validate()
            .map_err(|e| api(ErrorCode::StoreCorrupt, e))?;
        Ok(page)
    }
}
fn scope_matches(scope: &IntentScope, semantic: &SemanticMutation) -> bool {
    match (scope, semantic) {
        (IntentScope::Cooperative { instance, seat }, semantic) => semantic
            .frozen_claim()
            .is_some_and(|claim| &claim.instance == instance && &claim.seat == seat),
        (
            IntentScope::ServiceAllocation { target, .. },
            SemanticMutation::ResolveSeat { target: requested },
        ) => target == requested,
        (
            IntentScope::Continuity { target, .. },
            SemanticMutation::ContinuityCheckIn {
                target: requested, ..
            },
        ) => target == requested,
        (IntentScope::Native { .. }, request) => {
            !request.is_operator()
                && request.frozen_claim().is_none()
                && !matches!(
                    request,
                    SemanticMutation::ResolveSeat { .. }
                        | SemanticMutation::ContinuityCheckIn { .. }
                )
        }
        (IntentScope::Operator { .. }, request) => request.is_operator(),
        _ => false,
    }
}
fn set_cursor(
    page: &mut PendingPage,
    key: &str,
    request: &PageRequest,
    prefix: &[String],
    ordinal: u64,
    reason: StopReason,
) -> Result<(), ApiError> {
    let cursor = Cursor {
        instance: key.into(),
        scope: CursorScope::LocalIntents,
        scope_key: key.into(),
        filter_digest: key.into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: ordinal,
        high_water_ordinal: page.high_water_ordinal,
        scope_revision: None,
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    }
    .encode()
    .map_err(|e| api(ErrorCode::InvalidCursor, e))?;
    let mut argv = prefix.to_vec();
    argv.extend([
        "--cursor".into(),
        cursor.clone(),
        "--limit".into(),
        request.limit.to_string(),
        "--max-bytes".into(),
        request.max_bytes.to_string(),
    ]);
    page.next_argv = Some(argv);
    page.next_cursor = Some(cursor);
    page.has_more = true;
    page.stop_reason = reason;
    Ok(())
}
fn selected_len(page: &PendingPage, output: &OutputSpec) -> Result<usize, ApiError> {
    Ok(encode_selected(&CommandResult::LocalIntents(page.clone()), output)?.len())
}
fn budget_error(minimum: usize) -> ApiError {
    ApiError::invalid_budget("intent output budget too small")
        .with_required_minimum_bytes(u32::try_from(minimum).unwrap_or(u32::MAX))
}
fn api(code: ErrorCode, detail: impl Into<String>) -> ApiError {
    ApiError::new(code, detail)
}
fn io_api(error: impl std::fmt::Display) -> ApiError {
    api(ErrorCode::StoreCorrupt, error.to_string())
}
fn api_io(error: ApiError) -> io::Error {
    invalid(format!(
        "{}: {}",
        serde_json::to_string(&error.code).unwrap_or_default(),
        error.detail
    ))
}
fn validate_prefix(prefix: &[String], output: &OutputSpec) -> Result<(), ApiError> {
    if prefix.last().map(String::as_str) != Some("pending-ops") {
        return Err(api(
            ErrorCode::InvalidRequest,
            "missing pending-ops continuation command",
        ));
    }
    for flag in ["--state-dir", "--host-endpoint", "--json"] {
        if prefix.iter().filter(|part| part.as_str() == flag).count() > 1 {
            return Err(api(
                ErrorCode::InvalidRequest,
                "duplicate continuation selector",
            ));
        }
    }
    if prefix
        .iter()
        .any(|part| matches!(part.as_str(), "--cursor" | "--limit" | "--max-bytes"))
    {
        return Err(api(
            ErrorCode::InvalidRequest,
            "page flags are not part of continuation prefix",
        ));
    }
    let has_value = |flag: &str| {
        prefix
            .windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    };
    if has_value("--state-dir") != output.context.state_dir.as_deref()
        || has_value("--host-endpoint") != output.context.host.as_deref()
        || prefix.iter().any(|s| s == "--json") != (output.format == OutputFormat::Json)
    {
        return Err(api(
            ErrorCode::InvalidRequest,
            "continuation context differs from selected output",
        ));
    }
    Ok(())
}
fn invalid(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}
fn private_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}
/// Sandbox-writable (Codex) instance directory: never follow a leaf
/// symlink (TRUST-POLICY Accepted limits), like the context journal.
/// Header line of one intent file for a scan, or `None` for anything that is
/// not a readable intent: a vanished entry, a symlink, a FIFO or device (opened
/// without blocking and rejected by `is_file`), a read error or garbage.
fn read_intent_header(path: &Path) -> Option<IntentHeader> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut line = String::new();
    BufReader::new(file.take(64 * 1024))
        .read_line(&mut line)
        .ok()?;
    serde_json::from_str(&line).ok()
}

fn private_open(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    options.open(path)
}
#[cfg(test)]
#[path = "../../tests/cli/journal.rs"]
mod tests;
