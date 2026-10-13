//! Additive automatic contracts. Legacy handoff wire types remain independent.
use super::{HandoffChannel, HandoffNamespace, bounded_text};
use crate::{
    cli::journal::{IntentScope, OriginalActor, SemanticMutation, classify_original_actor},
    protocol::{
        authority::{CallerClaim, CallerRole},
        commands::{DeliveryMode, EnsureDirectThread, SendMessage},
        ids::{OperationId, SeatId, ThreadId},
        results::CommandResult,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const AUTOMATIC_PLAN_VERSION: u32 = 2;
pub(crate) const MAX_AUTOMATIC_ORIGINAL_BYTES: usize = 524288;

/// Automatic selection has one Ensure child and no independent invitation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticHandoffKeys {
    pub compound: OperationId,
    pub begin: OperationId,
    pub ensure: OperationId,
    pub send: OperationId,
    pub complete: OperationId,
}
impl AutomaticHandoffKeys {
    pub fn new(compound: OperationId) -> Self {
        let child = |phase: &str| {
            OperationId::new(format!(
                "automatic-{:x}",
                Sha256::digest(
                    format!("automatic-handoff-v2\0{}\0{phase}", compound.as_str()).as_bytes()
                )
            ))
        };
        Self {
            begin: child("begin"),
            ensure: child("ensure"),
            send: child("send"),
            complete: child("complete"),
            compound,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let keys = [
            &self.compound,
            &self.begin,
            &self.ensure,
            &self.send,
            &self.complete,
        ];
        if keys.iter().any(|key| !bounded_text(key.as_str(), 256))
            || keys
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != keys.len()
        {
            return Err("invalid or duplicate automatic child key");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticHandoffPayload {
    /// Omission preserves the released V2 original and its digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<super::HandoffPolicy>,
    pub namespace: HandoffNamespace,
    pub keys: AutomaticHandoffKeys,
    pub channel: HandoffChannel,
    pub body: String,
}
impl AutomaticHandoffPayload {
    pub fn new(
        namespace: HandoffNamespace,
        compound: OperationId,
        body: String,
    ) -> Result<Self, &'static str> {
        let payload = Self {
            policy: None,
            namespace,
            keys: AutomaticHandoffKeys::new(compound),
            channel: HandoffChannel::Automatic,
            body,
        };
        payload.validate()?;
        Ok(payload)
    }
    pub fn with_policy(
        mut self,
        policy: Option<super::HandoffPolicy>,
    ) -> Result<Self, &'static str> {
        self.policy = policy;
        self.validate()?;
        Ok(self)
    }
    pub fn creation_policy(&self) -> Option<crate::protocol::commands::EnsureCreationPolicy> {
        self.policy
            .as_ref()
            .map(|p| crate::protocol::commands::EnsureCreationPolicy {
                confirmation: p.invitation_confirmation,
                deadline_millis: p.invite_deadline_millis,
            })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(policy) = &self.policy {
            policy.validate()?;
        }
        self.namespace.validate()?;
        self.keys.validate()?;
        if self.channel != HandoffChannel::Automatic
            || self.body.is_empty()
            || self.body.len() > 65536
        {
            return Err("invalid automatic handoff payload");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticDeliveryPlan {
    pub version: u32,
    pub payload: AutomaticHandoffPayload,
    pub recipient: SeatId,
}
impl AutomaticDeliveryPlan {
    pub fn new(payload: AutomaticHandoffPayload, recipient: SeatId) -> Result<Self, &'static str> {
        let plan = Self {
            version: AUTOMATIC_PLAN_VERSION,
            payload,
            recipient,
        };
        plan.validate()?;
        Ok(plan)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != AUTOMATIC_PLAN_VERSION {
            return Err("unsupported automatic delivery version");
        }
        self.payload.validate()
    }
}
/// One original journal identity includes the native launch and routing context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticHandoffPlan {
    pub version: u32,
    pub delivery: AutomaticDeliveryPlan,
    #[serde(deserialize_with = "deserialize_exact")]
    pub launch: crate::cli::launch::LaunchRequest,
    #[serde(deserialize_with = "deserialize_exact")]
    pub context: crate::protocol::output::ContinuationContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_input: Option<crate::harness::adapter::StartupInputTemplate>,
}
impl AutomaticHandoffPlan {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != AUTOMATIC_PLAN_VERSION {
            return Err("unsupported automatic handoff version");
        }
        self.delivery.validate()?;
        if self.context.state_dir.as_deref() != self.delivery.payload.namespace.state_dir.to_str()
            || self.context.host.as_deref()
                != self.delivery.payload.namespace.host_endpoint.to_str()
        {
            return Err("automatic launch context differs from frozen namespace");
        }
        if self.launch.argv.len() > 256
            || self
                .launch
                .argv
                .iter()
                .any(|arg| arg.len() > 4096 || arg.contains('\0'))
            || self.launch.argv.iter().map(String::len).sum::<usize>() > 65536
        {
            return Err("invalid automatic launch arguments");
        }
        if let Some(template) = &self.startup_input {
            template
                .validate(self.launch.argv.len())
                .map_err(|_| "invalid automatic startup template")?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "plan",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AutomaticOriginal {
    Delivery(Box<AutomaticDeliveryPlan>),
    Handoff(Box<AutomaticHandoffPlan>),
}
impl AutomaticOriginal {
    /// Borrow the child directly; a second payload copy cannot disagree with the original.
    pub fn delivery(&self) -> &AutomaticDeliveryPlan {
        match self {
            Self::Delivery(plan) => plan,
            Self::Handoff(plan) => &plan.delivery,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Delivery(plan) => plan.validate(),
            Self::Handoff(plan) => plan.validate(),
        }
    }
    pub fn semantic(&self, claim: CallerClaim) -> SemanticMutation {
        SemanticMutation::Frozen {
            claim,
            mutation: Box::new(match self {
                Self::Delivery(plan) => SemanticMutation::AutomaticDelivery(plan.clone()),
                Self::Handoff(plan) => SemanticMutation::AutomaticHandoff(plan.clone()),
            }),
        }
    }
    pub fn digest(&self, claim: &CallerClaim) -> Result<String, &'static str> {
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&self.semantic(claim.clone()))
                    .map_err(|_| "invalid automatic original")?
            )
        ))
    }
}
/// No raw Create or Invite phase can be decoded for an automatic original.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "phase",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AutomaticDeliveryAction {
    Status,
    Prepare,
    Begin { operation: OperationId },
    Ensure(EnsureDirectThread),
    Send(SendMessage),
    Complete { operation: OperationId },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomaticDeliveryMutation {
    pub version: u32,
    pub scope: IntentScope,
    pub claim: CallerClaim,
    pub digest: String,
    #[serde(deserialize_with = "deserialize_exact")]
    pub original: AutomaticOriginal,
    pub action: AutomaticDeliveryAction,
}
impl AutomaticDeliveryMutation {
    pub fn new(
        scope: IntentScope,
        claim: CallerClaim,
        original: AutomaticOriginal,
    ) -> Result<Self, &'static str> {
        let digest = original.digest(&claim)?;
        let request = Self {
            version: AUTOMATIC_PLAN_VERSION,
            scope,
            claim,
            digest,
            original,
            action: AutomaticDeliveryAction::Prepare,
        };
        request.validate()?;
        Ok(request)
    }
    pub fn plan(&self) -> &AutomaticDeliveryPlan {
        self.original.delivery()
    }
    pub fn begin(&self) -> Self {
        let mut request = self.clone();
        request.action = AutomaticDeliveryAction::Begin {
            operation: self.plan().payload.keys.begin.clone(),
        };
        request
    }
    pub fn ensure(&self) -> Self {
        let mut request = self.clone();
        request.action = AutomaticDeliveryAction::Ensure(EnsureDirectThread {
            creation_policy: self.plan().payload.creation_policy(),
            recipient: self.plan().recipient.clone(),
            operation: self.plan().payload.keys.ensure.clone(),
            claim: self.claim.clone(),
        });
        request
    }
    /// This constructs an ordinary send; canonical attachment must decide thread equality.
    pub fn send(&self, thread: ThreadId) -> Self {
        let mut request = self.clone();
        request.action = AutomaticDeliveryAction::Send(SendMessage {
            priority: None,
            delivery_mode: DeliveryMode::Ordinary,
            thread,
            body: self.plan().payload.body.clone(),
            direct_recipient: Some(self.plan().recipient.clone()),
            invited_recipients: vec![self.plan().recipient.clone()],
            deadline_millis: self
                .plan()
                .payload
                .policy
                .as_ref()
                .and_then(|p| p.deadline_millis),
            operation: self.plan().payload.keys.send.clone(),
            claim: self.claim.clone(),
            relays_user: false,
            user_intent: None,
        });
        request
    }
    pub fn complete(&self) -> Self {
        let mut request = self.clone();
        request.action = AutomaticDeliveryAction::Complete {
            operation: self.plan().payload.keys.complete.clone(),
        };
        request
    }
    /// Prove every ordinary child continuation fits the unchanged wire cap.
    /// The escaped identifiers are sizing witnesses, never retained selections.
    pub(crate) fn validate_admission(&self) -> Result<(), &'static str> {
        use crate::protocol::wire::{MAX_WIRE_FRAME_BYTES, PROTOCOL_VERSION, WireRequest};
        if self.plan().payload.policy.is_none() {
            return Ok(());
        }
        self.validate()?;
        let thread = ThreadId::new("\\".repeat(128));
        for request in [
            self.clone(),
            self.begin(),
            self.ensure(),
            self.send(thread.clone()),
            self.complete(),
        ] {
            let wire = WireRequest {
                version: PROTOCOL_VERSION,
                request_id: "\\".repeat(128),
                expected_instance: "f".repeat(36),
                expected_boot: Some("f".repeat(36)),
                output: Some(crate::protocol::output::OutputSpec::default()),
                command: crate::protocol::commands::Command::AutomaticHandoffDelivery(Box::new(
                    request,
                )),
            };
            if serde_json::to_vec(&wire)
                .map_err(|_| "automatic request encoding")?
                .len()
                + 4
                > MAX_WIRE_FRAME_BYTES
            {
                return Err("automatic future request exceeds 1 MiB");
            }
        }
        self.admission_summaries()?;
        Ok(())
    }
    pub(crate) fn admission_summaries(&self) -> Result<Vec<serde_json::Value>, &'static str> {
        let thread = ThreadId::new("\\".repeat(128));
        let mut summaries = Vec::new();
        if let AutomaticOriginal::Handoff(plan) = &self.original {
            let template = plan
                .startup_input
                .as_ref()
                .ok_or("modern automatic origin lacks startup template")?;
            for prompt in crate::cli::handoff::bootstrap_v1::retained_prompts(
                thread.as_str(),
                &plan.context,
                &self.claim.instance,
            ) {
                let (argv, _) = template
                    .apply(&plan.launch.argv, &prompt)
                    .map_err(|_| "automatic future startup input exceeds bounds")?;
                let argv = crate::harness::launch::compose_native_argv(
                    plan.launch.harness.into(),
                    argv,
                    vec![],
                )
                .map_err(|_| "automatic future native argv exceeds bounds")?;
                crate::ports::NativeLaunchRequest::validate_argv(&argv)?;
                let report = serde_json::json!({"outcome":"started","pane":"\\".repeat(128),"seat":plan.delivery.recipient,"harness":plan.launch.harness.as_str(),"argv":argv});
                summaries.push(super::automatic_launch_summary(&report, &"f".repeat(64))?);
            }
        }
        if summaries.is_empty() {
            summaries.push(serde_json::Value::Null);
        }
        Ok(summaries)
    }
    /// The canonical representation stores the entire normalized original,
    /// including its frozen caller and outer launch. Escaping counts toward the bound.
    pub(crate) fn canonical_original_bytes(&self) -> Result<Vec<u8>, &'static str> {
        let mut normalized = self.clone();
        normalized.action = AutomaticDeliveryAction::Prepare;
        let bytes = serde_json::to_vec(&normalized).map_err(|_| "invalid automatic original")?;
        let limit = if self.plan().payload.policy.is_some() {
            MAX_AUTOMATIC_ORIGINAL_BYTES
        } else {
            128 * 1024
        };
        if bytes.len() > limit {
            return Err("automatic original exceeds canonical storage bound");
        }
        Ok(bytes)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.original.validate()?;
        self.canonical_original_bytes()?;
        let plan = self.plan();
        let keys = &plan.payload.keys;
        if self.version != AUTOMATIC_PLAN_VERSION
            || self.claim.role != CallerRole::TopLevel
            || self.claim.instance != plan.payload.namespace.instance
            || self.claim.seat == plan.recipient
            || self.digest != self.original.digest(&self.claim)?
            || classify_original_actor(&self.scope, &self.original.semantic(self.claim.clone()))
                .map_err(|_| "invalid automatic original actor")?
                != OriginalActor::Agent
        {
            return Err("automatic original identity mismatch");
        }
        let exact = match &self.action {
            AutomaticDeliveryAction::Status | AutomaticDeliveryAction::Prepare => true,
            AutomaticDeliveryAction::Begin { operation } => operation == &keys.begin,
            AutomaticDeliveryAction::Complete { operation } => operation == &keys.complete,
            AutomaticDeliveryAction::Ensure(value) => {
                value.claim == self.claim
                    && value.operation == keys.ensure
                    && value.recipient == plan.recipient
                    && value.creation_policy == plan.payload.creation_policy()
            }
            AutomaticDeliveryAction::Send(value) => {
                value.claim == self.claim
                    && value.operation == keys.send
                    && value.delivery_mode.is_ordinary()
                    && value.body == plan.payload.body
                    && value.direct_recipient.as_ref() == Some(&plan.recipient)
                    && value.invited_recipients == [plan.recipient.clone()]
                    && value.deadline_millis
                        == plan.payload.policy.as_ref().and_then(|p| p.deadline_millis)
                    && !value.relays_user
                    && value.user_intent.is_none()
            }
        };
        if !exact {
            return Err("automatic phase differs from frozen original");
        }
        Ok(())
    }
}
/// Retain the canonical result; names and presentation metadata cannot select a pair.
pub fn automatic_selected_thread(result: &CommandResult) -> Result<&ThreadId, &'static str> {
    match result {
        CommandResult::ThreadCreated(thread) => Ok(thread),
        CommandResult::DirectThreadEnsured(actual) => {
            actual.validate()?;
            Ok(&actual.thread)
        }
        CommandResult::Handoff(status) => status
            .thread
            .as_ref()
            .ok_or("automatic fence has no attached thread"),
        _ => Err("automatic selection returned an unexpected result"),
    }
}

pub(super) fn deserialize_exact<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Serialize,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let decoded: T = serde_json::from_value(value.clone()).map_err(serde::de::Error::custom)?;
    if value != serde_json::to_value(&decoded).map_err(serde::de::Error::custom)? {
        return Err(serde::de::Error::custom(
            "automatic wire contains lost fields",
        ));
    }
    Ok(decoded)
}
