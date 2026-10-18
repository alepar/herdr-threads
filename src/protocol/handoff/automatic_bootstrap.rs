//! V2 topology original and exact derived automatic child, independent of V1.
use super::*;
use crate::{
    cli::journal::{IntentScope, OriginalActor, SemanticMutation, classify_original_claim},
    ports::CreatedTab,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticBootstrapKeys {
    pub compound: OperationId,
    pub begin: OperationId,
    pub resolve: OperationId,
    pub attach: OperationId,
    pub complete: OperationId,
}
impl AutomaticBootstrapKeys {
    pub fn new(compound: OperationId) -> Self {
        let key = |phase: &str| {
            OperationId::new(format!(
                "automatic-bootstrap-{:x}",
                Sha256::digest(format!(
                    "automatic-bootstrap-v2\0{}\0{phase}",
                    compound.as_str()
                ))
            ))
        };
        Self {
            begin: key("begin"),
            resolve: key("resolve"),
            attach: key("attach"),
            complete: key("complete"),
            compound,
        }
    }
    pub fn all(&self) -> [&OperationId; 5] {
        [
            &self.compound,
            &self.begin,
            &self.resolve,
            &self.attach,
            &self.complete,
        ]
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticBootstrapPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory_context: Option<LaunchDirectoryContext>,
    pub handoff: AutomaticHandoffPayload,
    pub keys: AutomaticBootstrapKeys,
    pub workspace: HostTargetId,
    pub cwd: std::path::PathBuf,
    pub label: String,
    pub focus: bool,
    pub env: std::collections::BTreeMap<String, String>,
    pub launch: BootstrapLaunch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_input: Option<crate::harness::adapter::StartupInputTemplate>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticBootstrapPlan {
    pub version: u32,
    pub payload: AutomaticBootstrapPayload,
}
impl AutomaticBootstrapPlan {
    pub fn validate(&self) -> Result<(), &'static str> {
        let p = &self.payload;
        if let Some(context) = &p.directory_context {
            context.validate_bootstrap(&p.cwd, &p.env)?;
        }
        p.handoff.validate()?;
        p.launch.validate()?;
        if self.version != 2
            || !frozen_absolute_path(&p.cwd)
            || !bounded_text(&p.label, 128)
            || p.label.chars().any(char::is_control)
            || p.focus
            || (p.directory_context.is_none() && !p.env.is_empty())
        {
            return Err("invalid automatic bootstrap topology");
        }
        if let Some(template) = &p.startup_input {
            template
                .validate(p.launch.argv.len())
                .map_err(|_| "invalid bootstrap startup template")?;
        }
        let c = &p.handoff.keys;
        let mut keys = p.keys.all().to_vec();
        keys.extend([&c.compound, &c.begin, &c.ensure, &c.send, &c.complete]);
        if keys.iter().any(|k| !bounded_text(k.as_str(), 256))
            || keys
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != keys.len()
        {
            return Err("duplicate or invalid automatic bootstrap key");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticBootstrapIdentity {
    pub version: u32,
    pub scope: IntentScope,
    pub claim: CallerClaim,
    pub digest: String,
    pub plan: AutomaticBootstrapPlan,
}
impl AutomaticBootstrapIdentity {
    pub fn new(
        scope: IntentScope,
        claim: CallerClaim,
        plan: AutomaticBootstrapPlan,
    ) -> Result<Self, &'static str> {
        let mut id = Self {
            version: 2,
            scope,
            claim,
            digest: String::new(),
            plan,
        };
        id.digest = id.semantic_digest()?;
        id.validate()?;
        Ok(id)
    }
    pub fn semantic(&self) -> SemanticMutation {
        SemanticMutation::Frozen {
            claim: self.claim.clone(),
            mutation: Box::new(SemanticMutation::AutomaticBootstrap(Box::new(
                self.plan.clone(),
            ))),
        }
    }
    pub fn semantic_digest(&self) -> Result<String, &'static str> {
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&self.semantic())
                    .map_err(|_| "invalid automatic bootstrap semantic")?
            )
        ))
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.plan.validate()?;
        if self.version != 2
            || self.claim.role != crate::protocol::authority::CallerRole::TopLevel
            || self.claim.instance != self.plan.payload.handoff.namespace.instance
            || self.claim.host.as_path()
                != self.plan.payload.handoff.namespace.host_endpoint.as_path()
            || self.digest != self.semantic_digest()?
            || classify_original_claim(&self.scope, &self.claim)
                .map_err(|_| "invalid bootstrap scope")?
                != OriginalActor::Agent
        {
            return Err("automatic bootstrap original differs");
        }
        if serde_json::to_vec(self)
            .map_err(|_| "invalid bootstrap identity")?
            .len()
            > if self.plan.payload.handoff.policy.is_some() {
                automatic::MAX_AUTOMATIC_ORIGINAL_BYTES
            } else {
                131072
            }
        {
            return Err("automatic bootstrap original exceeds retained capacity");
        }
        Ok(())
    }
    /// Deterministic derivation from frozen inputs; never consult current defaults.
    pub fn child(
        &self,
        created: &CreatedTab,
        recipient: SeatId,
    ) -> Result<AutomaticDeliveryMutation, &'static str> {
        self.validate()?;
        created.validate()?;
        let p = &self.plan.payload;
        let ns = &p.handoff.namespace;
        if created.workspace != p.workspace
            || created.witness.endpoint.as_os_str() != ns.host_endpoint.as_os_str()
        {
            return Err("automatic bootstrap creation scope differs");
        }
        let harness = p.launch.harness.into();
        AutomaticDeliveryMutation::new(
            self.scope.clone(),
            self.claim.clone(),
            AutomaticOriginal::Handoff(Box::new(AutomaticHandoffPlan {
                version: 2,
                delivery: AutomaticDeliveryPlan::new(p.handoff.clone(), recipient)?,
                launch: crate::cli::launch::LaunchRequest {
                    directory_context: p.directory_context.clone(),
                    target: created.root_pane.clone(),
                    harness,
                    harness_binary: p.launch.binary.clone(),
                    argv: p.launch.argv.clone(),
                    name: p.launch.name.clone(),
                    pane_label: Some(p.label.clone()),
                },
                context: crate::protocol::output::ContinuationContext {
                    state_dir: ns.state_dir.to_str().map(str::to_owned),
                    host: ns.host_endpoint.to_str().map(str::to_owned),
                },
                startup_input: p.startup_input.clone(),
            })),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticBootstrapAttachment {
    pub attempt: BootstrapAttempt,
    pub created: CreatedTab,
    pub resolve_operation: OperationId,
    pub resolved_seat: SeatId,
    pub child: AutomaticDeliveryMutation,
}
impl AutomaticBootstrapAttachment {
    pub fn derive(
        id: &AutomaticBootstrapIdentity,
        attempt: BootstrapAttempt,
        created: CreatedTab,
        recipient: SeatId,
    ) -> Result<Self, &'static str> {
        let child = id.child(&created, recipient.clone())?;
        let attachment = Self {
            attempt,
            created,
            resolve_operation: id.plan.payload.keys.resolve.clone(),
            resolved_seat: recipient,
            child,
        };
        attachment.validate(id)?;
        Ok(attachment)
    }
    pub fn validate(&self, id: &AutomaticBootstrapIdentity) -> Result<(), &'static str> {
        let expected = id.child(&self.created, self.resolved_seat.clone())?;
        if self.resolve_operation != id.plan.payload.keys.resolve
            || serde_json::to_vec(&self.child).map_err(|_| "invalid attachment")?
                != serde_json::to_vec(&expected).map_err(|_| "invalid derived child")?
        {
            return Err("automatic bootstrap exact derived child differs");
        }
        if serde_json::to_vec(self)
            .map_err(|_| "invalid automatic attachment")?
            .len()
            > if id.plan.payload.handoff.policy.is_some() {
                automatic::MAX_AUTOMATIC_ORIGINAL_BYTES
            } else {
                131072
            }
        {
            return Err("automatic attachment exceeds its retained capacity");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "phase",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AutomaticBootstrapAction {
    Status,
    Begin,
    Reserve {
        attempt: BootstrapAttempt,
    },
    CheckSubmission {
        attempt: BootstrapAttempt,
        administrative_revision: u64,
    },
    RecordCreated {
        attempt: BootstrapAttempt,
        evidence: CreatedTab,
    },
    RecordOutcomeUnknown {
        attempt: BootstrapAttempt,
    },
    Recover {
        attempt: BootstrapAttempt,
        inspection: BootstrapRecoveryInspection,
        disposition: AutomaticBootstrapRecoveryDisposition,
    },
    RecordNotSubmitted {
        attempt: BootstrapAttempt,
    },
    Resolve {
        attempt: BootstrapAttempt,
    },
    Attach(Box<AutomaticBootstrapAttachment>),
    CheckStart,
    Complete {
        attachment: Box<AutomaticBootstrapAttachment>,
        retained: LinkedBootstrapReport,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticBootstrapMutation {
    #[serde(deserialize_with = "automatic::deserialize_exact")]
    pub identity: AutomaticBootstrapIdentity,
    #[serde(deserialize_with = "automatic::deserialize_exact")]
    pub action: AutomaticBootstrapAction,
}
impl AutomaticBootstrapMutation {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.identity.validate()?;
        match &self.action {
            AutomaticBootstrapAction::Attach(a) => a.validate(&self.identity)?,
            AutomaticBootstrapAction::RecordCreated { evidence, .. } => {
                self.identity
                    .child(evidence, SeatId::new("validation-recipient"))?;
            }
            AutomaticBootstrapAction::Complete {
                attachment,
                retained,
            } => {
                attachment.validate(&self.identity)?;
                retained.validate()?;
                if serde_json::to_vec(self)
                    .map_err(|_| "invalid automatic completion")?
                    .len()
                    > 2097152
                {
                    return Err("automatic completion exceeds 2 MiB");
                }
                let p = &self.identity.plan.payload;
                if retained.launch != p.launch
                    || retained.recipient != attachment.resolved_seat
                    || retained.pane != attachment.created.root_pane
                    || retained.terminal != attachment.created.terminal
                    || retained.host_incarnation != attachment.created.host_incarnation
                {
                    return Err("automatic retained launch differs");
                }
            }
            AutomaticBootstrapAction::Recover {
                inspection,
                disposition,
                ..
            } => {
                inspection.validate()?;
                disposition.validate()?;
                if serde_json::to_vec(self)
                    .map_err(|_| "invalid automatic recovery")?
                    .len()
                    > if self.identity.plan.payload.handoff.policy.is_some() {
                        1048572
                    } else {
                        262144
                    }
                {
                    return Err("automatic recovery exceeds retained request limit");
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn operation(&self) -> Result<OperationId, &'static str> {
        let k = &self.identity.plan.payload.keys;
        match &self.action {
            AutomaticBootstrapAction::Begin => Ok(k.begin.clone()),
            AutomaticBootstrapAction::Resolve { .. } => Ok(k.resolve.clone()),
            AutomaticBootstrapAction::Attach(_) => Ok(k.attach.clone()),
            AutomaticBootstrapAction::Complete { .. } => Ok(k.complete.clone()),
            AutomaticBootstrapAction::Reserve { attempt } => {
                attempt.operation(&k.compound, "reserve")
            }
            AutomaticBootstrapAction::CheckSubmission { attempt, .. } => {
                attempt.operation(&k.compound, "check")
            }
            AutomaticBootstrapAction::RecordCreated { attempt, .. } => {
                attempt.operation(&k.compound, "record")
            }
            AutomaticBootstrapAction::RecordOutcomeUnknown { attempt } => {
                Ok(automatic_unknown_key(&k.compound, *attempt))
            }
            AutomaticBootstrapAction::Recover {
                attempt,
                inspection,
                disposition,
            } => OperationId::parse(format!(
                "automatic-recovery-{:x}",
                Sha256::digest(
                    serde_json::to_vec(&(
                        "automatic-bootstrap-recovery-v2",
                        &self.identity.digest,
                        &k.compound,
                        attempt,
                        inspection,
                        disposition
                    ))
                    .map_err(|_| "invalid recovery decision")?
                )
            )),
            AutomaticBootstrapAction::RecordNotSubmitted { attempt } => {
                attempt.operation(&k.compound, "not_submitted")
            }
            _ => Err("automatic bootstrap query has no operation"),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticBootstrapResult {
    pub identity: AutomaticBootstrapIdentity,
    pub state: BootstrapState,
    pub attempt: BootstrapAttempt,
    pub attempt_state: BootstrapAttemptState,
    pub administrative_revision: u64,
    pub creation: Option<CreatedTab>,
    pub attachment: Option<AutomaticBootstrapAttachment>,
    pub thread: Option<ThreadId>,
    pub completed: Option<LinkedBootstrapReport>,
    pub recovery: Option<AutomaticBootstrapRecoveryResult>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticBootstrapReserved {
    pub status: AutomaticBootstrapResult,
    pub authorization: Option<BootstrapSubmissionAuthorization>,
}

pub(crate) fn automatic_unknown_key(
    compound: &OperationId,
    attempt: BootstrapAttempt,
) -> OperationId {
    OperationId::new(format!(
        "automatic-unknown-{:x}",
        Sha256::digest(format!("{}\0{}\0unknown", compound.as_str(), attempt.get()))
    ))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AutomaticBootstrapRecoveryDisposition {
    CreatedPane {
        evidence: Box<CreatedTab>,
        structural_reference: HostCallId,
    },
    NotCreated {
        quiescence: BootstrapQuiescenceAssertion,
    },
    Cancelled {
        reason: String,
        quiescence: BootstrapQuiescenceAssertion,
        attached_child: Option<Box<AutomaticDeliveryMutation>>,
    },
}
impl AutomaticBootstrapRecoveryDisposition {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::CreatedPane { evidence, .. } => evidence.validate(),
            Self::NotCreated {
                quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
            } => Ok(()),
            Self::NotCreated { .. } => {
                Err("not-created requires inspected noncreation and quiescence")
            }
            Self::Cancelled {
                reason,
                attached_child,
                ..
            } => {
                if !bounded_text(reason, 4096) {
                    return Err("invalid cancellation reason");
                }
                if let Some(child) = attached_child {
                    child.validate()?;
                }
                Ok(())
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticBootstrapRecoveryResult {
    pub operation: OperationId,
    pub attempt: BootstrapAttempt,
    pub inspection: BootstrapRecoveryInspection,
    pub disposition: AutomaticBootstrapRecoveryDisposition,
    pub operator_uid: u32,
    pub operator_provenance: String,
    pub state: BootstrapState,
    pub administrative_revision: u64,
}
impl AutomaticBootstrapResult {
    pub fn recovery_inspection(&self) -> Result<BootstrapRecoveryInspection, &'static str> {
        let bytes = serde_json::to_vec(&(
            "automatic-bootstrap-inspection-v2",
            &self.identity.digest,
            self.state,
            self.attempt,
            self.attempt_state,
            self.administrative_revision,
            &self.creation,
            &self.attachment,
            &self.recovery,
        ))
        .map_err(|_| "invalid automatic inspection")?;
        if bytes.len()
            > if self.identity.plan.payload.handoff.policy.is_some() {
                2097152
            } else {
                524288
            }
        {
            return Err("oversized automatic inspection");
        }
        Ok(BootstrapRecoveryInspection {
            version: 1,
            digest: format!("{:x}", Sha256::digest(bytes)),
        })
    }
}

/// Modern automatic completion deliberately retains a bounded representation.
/// The separately named digest refers to the exact optional LOCAL diagnostic artifact.
pub(crate) fn automatic_launch_summary(
    report: &serde_json::Value,
    full_report_digest: &str,
) -> Result<serde_json::Value, &'static str> {
    if report["outcome"] != "started" || !super::valid_digest(full_report_digest) {
        return Err("automatic launch has no successful bounded summary");
    }
    for key in ["pane", "seat", "harness"] {
        if report[key].as_str().is_none_or(|v| !bounded_text(v, 128)) {
            return Err("invalid automatic launch summary identity");
        }
    }
    let argv = report["argv"]
        .as_array()
        .ok_or("missing automatic summary argv")?;
    if argv.is_empty()
        || argv.len() > 256
        || argv.iter().any(|v| {
            v.as_str()
                .is_none_or(|a| a.len() > 4096 || a.contains('\0'))
        })
    {
        return Err("invalid automatic launch summary argv");
    }
    Ok(serde_json::json!({
        "format":"automatic_launch_summary_v1", "outcome":"started",
        "pane":report["pane"], "seat":report["seat"], "harness":report["harness"],
        "argv":report["argv"], "full_report_digest":full_report_digest,
        "diagnostics":"optional_local_artifact",
    }))
}

impl AutomaticBootstrapIdentity {
    /// Admission covers every producer-generated continuation, before publication.
    /// Synthetic values below use the enforced ID/path/numeric bounds; they are
    /// sizing witnesses, never creation evidence or retained execution claims.
    pub(crate) fn validate_admission(&self) -> Result<(), &'static str> {
        self.validate_admission_for_local(None)
    }
    pub(crate) fn validate_local_admission(
        &self,
        root: &std::path::Path,
    ) -> Result<(), &'static str> {
        self.validate_admission_for_local(Some(root))
    }
    fn validate_admission_for_local(
        &self,
        root: Option<&std::path::Path>,
    ) -> Result<(), &'static str> {
        use crate::host::continuity::{
            LocalEndpointWitness, MACOS_WITNESS_PLATFORM, SocketIdentity,
        };
        use crate::protocol::{commands::Command, ids::MessageId, results::CommandResult, wire::*};
        if self.plan.payload.handoff.policy.is_none() {
            return Ok(());
        }
        self.validate()?;
        let p = &self.plan.payload;
        let escaped = "\\".repeat(128);
        let thread = ThreadId::new(escaped.clone());
        let recipient = SeatId::new(escaped.clone());
        let prefix = format!("{}:", p.workspace.as_str());
        if prefix.len() >= 128 {
            return Err("automatic workspace leaves no bounded pane identifier");
        }
        let target = |last: char| {
            HostTargetId::new(format!(
                "{}{}{}",
                prefix,
                "\\".repeat(127 - prefix.len()),
                last
            ))
        };
        let witness = LocalEndpointWitness {
            schema: 1,
            platform: MACOS_WITNESS_PLATFORM.into(),
            endpoint: p.handoff.namespace.host_endpoint.clone(),
            peer_uid: u32::MAX,
            peer_pid: u32::MAX,
            start_seconds: u64::MAX,
            start_microseconds: 999999,
            socket: SocketIdentity {
                device: u64::MAX,
                inode: u64::MAX,
                birth_seconds: i64::MIN,
                birth_nanoseconds: i64::MIN,
                change_seconds: i64::MIN,
                change_nanoseconds: i64::MIN,
            },
        };
        let created = CreatedTab {
            correlation: HostCallId::new(escaped.clone()),
            workspace: p.workspace.clone(),
            tab: target('t'),
            root_pane: target('p'),
            terminal: TerminalId::new(escaped.clone()),
            host_incarnation: HostBootId::new(format!(
                "herdr-server:pid={}:start={}.{:06}:uid={}",
                witness.peer_pid,
                witness.start_seconds,
                witness.start_microseconds,
                witness.peer_uid
            )),
            witness,
        };
        let attempt = BootstrapAttempt::new(u32::MAX)?;
        let attachment = AutomaticBootstrapAttachment::derive(
            self,
            attempt,
            created.clone(),
            recipient.clone(),
        )?;
        let context = crate::protocol::output::ContinuationContext {
            state_dir: p.handoff.namespace.state_dir.to_str().map(str::to_owned),
            host: p
                .handoff
                .namespace
                .host_endpoint
                .to_str()
                .map(str::to_owned),
        };
        let template = p
            .startup_input
            .as_ref()
            .ok_or("modern automatic origin lacks startup template")?;
        // Both canonical-routing and historical null-routing prompt outcomes are supported.
        let mut summaries = Vec::new();
        for prompt in crate::cli::handoff::bootstrap_v1::retained_prompts(
            thread.as_str(),
            &context,
            &self.claim.instance,
        ) {
            let (argv, _) = template
                .apply(&p.launch.argv, &prompt)
                .map_err(|_| "automatic future startup input exceeds bounds")?;
            let argv = crate::harness::launch::compose_native_argv(p.launch.harness, argv, vec![])
                .map_err(|_| "automatic future native argv exceeds bounds")?;
            crate::ports::NativeLaunchRequest::validate_argv(&argv)?;
            let source = serde_json::json!({"outcome":"started","pane":created.root_pane,"seat":recipient,"harness":p.launch.harness.as_str(),"argv":argv});
            summaries.push(automatic_launch_summary(&source, &"f".repeat(64))?);
        }
        let selection = crate::protocol::results::DirectThreadEnsured {
            thread: thread.clone(),
            recipient: recipient.clone(),
            outcome: crate::protocol::results::DirectThreadEnsureOutcome::ExistingPending,
            invitation: Some(crate::protocol::results::DirectThreadInvitation {
                id: crate::protocol::ids::InvitationId::new(escaped.clone()),
                episode: u64::MAX,
                confirmation: crate::protocol::commands::InvitationConfirmation::RequireAck,
                effective_deadline_at: Some(crate::protocol::time::UtcMillis(i64::MIN)),
                inviter: Some(SeatId::new(escaped.clone())),
            }),
        };
        let selection_response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: escaped.clone(),
            instance: "f".repeat(36),
            daemon_boot: "f".repeat(36),
            result: Ok(CommandResult::DirectThreadEnsured(selection.clone())),
        };
        if serde_json::to_vec(&selection_response)
            .map_err(|_| "automatic Ensure response encoding")?
            .len()
            + 4
            > MAX_WIRE_FRAME_BYTES
        {
            return Err("automatic Ensure response exceeds 1 MiB");
        }
        let inspection = BootstrapRecoveryInspection {
            version: 1,
            digest: "f".repeat(64),
        };
        let dispositions = vec![
            AutomaticBootstrapRecoveryDisposition::CreatedPane {
                evidence: Box::new(created.clone()),
                structural_reference: HostCallId::new(escaped.clone()),
            },
            AutomaticBootstrapRecoveryDisposition::NotCreated {
                quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
            },
            AutomaticBootstrapRecoveryDisposition::Cancelled {
                reason: format!("x{}", "\u{1}".repeat(4095)),
                quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                attached_child: Some(Box::new(attachment.child.clone())),
            },
        ];
        let wire_request = |command: Command| WireRequest {
            version: PROTOCOL_VERSION,
            request_id: escaped.clone(),
            expected_instance: "f".repeat(36),
            expected_boot: Some("f".repeat(36)),
            output: Some(crate::protocol::output::OutputSpec::default()),
            command,
        };
        let check_request = |command| -> Result<(), &'static str> {
            if serde_json::to_vec(&wire_request(command))
                .map_err(|_| "automatic future request encoding")?
                .len()
                + 4
                > MAX_WIRE_FRAME_BYTES
            {
                Err("automatic future request exceeds 1 MiB")
            } else {
                Ok(())
            }
        };
        let mutation = |action| {
            Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
                identity: self.clone(),
                action,
            }))
        };
        for action in [
            AutomaticBootstrapAction::Status,
            AutomaticBootstrapAction::Begin,
            AutomaticBootstrapAction::Reserve { attempt },
            AutomaticBootstrapAction::CheckSubmission {
                attempt,
                administrative_revision: u64::MAX,
            },
            AutomaticBootstrapAction::RecordCreated {
                attempt,
                evidence: created.clone(),
            },
            AutomaticBootstrapAction::RecordOutcomeUnknown { attempt },
            AutomaticBootstrapAction::RecordNotSubmitted { attempt },
            AutomaticBootstrapAction::Resolve { attempt },
            AutomaticBootstrapAction::Attach(Box::new(attachment.clone())),
            AutomaticBootstrapAction::CheckStart,
        ] {
            check_request(mutation(action))?;
        }
        for child in [
            attachment.child.clone(),
            attachment.child.begin(),
            attachment.child.ensure(),
            attachment.child.send(thread.clone()),
            attachment.child.complete(),
        ] {
            check_request(Command::AutomaticHandoffDelivery(Box::new(child)))?;
        }
        for disposition in &dispositions {
            check_request(mutation(AutomaticBootstrapAction::Recover {
                attempt,
                inspection: inspection.clone(),
                disposition: disposition.clone(),
            }))?;
        }
        for summary in summaries {
            let retained = LinkedBootstrapReport {
                thread: thread.clone(),
                recipient: recipient.clone(),
                pane: created.root_pane.clone(),
                kind: "launch".into(),
                launch: p.launch.clone(),
                report_digest: format!(
                    "{:x}",
                    Sha256::digest(
                        serde_json::to_vec(&summary).map_err(|_| "automatic summary encoding")?
                    )
                ),
                report: summary.clone(),
                terminal: created.terminal.clone(),
                host_incarnation: created.host_incarnation.clone(),
            };
            retained.validate()?;
            check_request(mutation(AutomaticBootstrapAction::Complete {
                attachment: Box::new(attachment.clone()),
                retained: retained.clone(),
            }))?;
            for disposition in &dispositions {
                let result = AutomaticBootstrapResult {
                    identity: self.clone(),
                    state: BootstrapState::PossibleCreation,
                    attempt,
                    attempt_state: BootstrapAttemptState::PossibleCreation,
                    administrative_revision: u64::MAX,
                    creation: Some(created.clone()),
                    attachment: Some(attachment.clone()),
                    thread: Some(thread.clone()),
                    completed: Some(retained.clone()),
                    recovery: Some(AutomaticBootstrapRecoveryResult {
                        operation: OperationId::new(escaped.clone()),
                        attempt,
                        inspection: inspection.clone(),
                        disposition: disposition.clone(),
                        operator_uid: u32::MAX,
                        operator_provenance: format!("operator:local-user:{}", u32::MAX),
                        state: BootstrapState::PossibleCreation,
                        administrative_revision: u64::MAX,
                    }),
                };
                // Optional fields are jointly present to dominate every reachable result.
                for result in [
                    CommandResult::AutomaticBootstrap(Box::new(result.clone())),
                    CommandResult::AutomaticBootstrapReserved(Box::new(
                        AutomaticBootstrapReserved {
                            status: result.clone(),
                            authorization: Some(BootstrapSubmissionAuthorization {
                                compound: p.keys.compound.clone(),
                                attempt,
                                reservation: OperationId::new(escaped.clone()),
                                administrative_revision: u64::MAX,
                            }),
                        },
                    )),
                ] {
                    let response = WireResponse {
                        version: PROTOCOL_VERSION,
                        request_id: escaped.clone(),
                        instance: "f".repeat(36),
                        daemon_boot: "f".repeat(36),
                        result: Ok(result),
                    };
                    if serde_json::to_vec(&response)
                        .map_err(|_| "automatic future reply encoding")?
                        .len()
                        + 4
                        > MAX_AUTOMATIC_RESPONSE_FRAME_BYTES
                    {
                        return Err("automatic future response exceeds 2 MiB");
                    }
                }
                let message = CommandResult::MessageSent(MessageId::new(escaped.clone()));
                for local in [
                    serde_json::json!({"version":2,"identity":self,"attachment":attachment,"thread":thread,"selection":selection,"message":message,"possible_start":true,"launch":summary}),
                    serde_json::json!({"version":2,"identity":self,"attempt":attempt,"possible_creation":true,"request":crate::ports::CreateTabRequest {correlation:created.correlation.clone(),workspace:p.workspace.clone(),cwd:p.cwd.clone(),label:p.label.clone(),focus:false,env:p.env.clone(),directory_context:p.directory_context.clone(),expected_witness:created.witness.clone()},"creation":created,"not_submitted":false}),
                ] {
                    if serde_json::to_vec(&local)
                        .map_err(|_| "automatic local envelope encoding")?
                        .len()
                        > crate::cli::topology_handoff::MAX_LINKED_LOCAL_BYTES
                    {
                        return Err("automatic future local progress exceeds 2 MiB");
                    }
                }
                if let Some(root) = root {
                    crate::cli::handoff::validate_automatic_presentation_admission(
                        root,
                        &p.handoff.namespace,
                        &summary,
                        &selection,
                        serde_json::json!({"creation":created,"bootstrap_compound":p.keys.compound,"attempt":attempt,"possible_start":true,"status_is_last_observed":true,"status_unknown":false}),
                    )?;
                }
                // The origin's header repeats the bounded original claim/scope. Use
                // its enforced envelope ceiling rather than a fixture header size.
                let original = serde_json::to_string(&self.semantic())
                    .map_err(|_| "automatic origin encoding")?;
                let terminal = serde_json::json!({"version":2,"original":original,"completed":result,"selection":selection,"message":message});
                if serde_json::to_vec(&terminal)
                    .map_err(|_| "automatic terminal encoding")?
                    .len()
                    + crate::cli::journal::MAX_AUTOMATIC_ORIGIN_BYTES
                    > crate::cli::topology_handoff::MAX_BOOTSTRAP_TERMINAL_BYTES
                {
                    return Err("automatic future terminal exceeds retained limit");
                }
            }
        }
        Ok(())
    }
}
