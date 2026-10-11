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
        p.handoff.validate()?;
        p.launch.validate()?;
        if self.version != 2
            || !frozen_absolute_path(&p.cwd)
            || !bounded_text(&p.label, 128)
            || p.label.chars().any(char::is_control)
            || p.focus
            || !p.env.is_empty()
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
            > automatic::MAX_AUTOMATIC_ORIGINAL_BYTES
        {
            return Err("automatic bootstrap original exceeds 128 KiB");
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
            > 131072
        {
            return Err("automatic attachment exceeds 128 KiB");
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
                    > 262144
                {
                    return Err("automatic recovery exceeds 256 KiB");
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
        if bytes.len() > 524288 {
            return Err("oversized automatic inspection");
        }
        Ok(BootstrapRecoveryInspection {
            version: 1,
            digest: format!("{:x}", Sha256::digest(bytes)),
        })
    }
}
