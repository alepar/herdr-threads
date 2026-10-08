//! Attempt-qualified administrative assertions; public execution remains inert.
pub const RECOVERY_HELP: &str = "Administrative recovery: ht human [GLOBALS] handoff recover REF --attempt N with exactly one --created-pane EXACT_PANE, --not-created, or --cancel --reason TEXT. Noncreation asserts inspected noncreation and quiescence; cancellation asserts quiescence. Reason must be nonblank and at most 4096 UTF-8 bytes. Recovery never launches or delivers downstream work. Public execution requires integrated canonical guards.";
use crate::protocol::{
    handoff::BootstrapAttempt,
    ids::{HostTargetId, LocalRecoveryRef},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub reference: LocalRecoveryRef,
    pub attempt: BootstrapAttempt,
    pub disposition: Assertion,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assertion {
    CreatedPane(HostTargetId),
    NotCreated,
    Cancelled { reason: String },
}

use super::{
    RunError,
    journal::{IntentRef, IntentScope, Journal, OriginalActor, SemanticMutation},
};
use crate::protocol::{
    commands::Command,
    handoff::{
        BootstrapIdentity, BootstrapRecoveryDisposition, HandoffNamespace, RecoverBootstrap,
    },
    output::OutputSpec,
    results::CommandResult,
    time::Clock,
};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};

/// This decision's caller is the local account. `identity.claim` belongs only
/// to the referenced original bootstrap, never to this operator action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPlan {
    pub version: u32,
    pub original_ref: IntentRef,
    pub operator_uid: u32,
    pub request: RecoverBootstrap,
}
impl RecoveryPlan {
    pub fn validate(&self) -> io::Result<()> {
        if self.version != 1 || self.original_ref.ordinal == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid recovery plan version or original reference",
            ));
        }
        let original = &self.request.identity;
        let semantic = SemanticMutation::freeze(
            SemanticMutation::HandoffBootstrap(Box::new(super::journal::BootstrapPlan {
                version: 1,
                payload: original.payload.clone(),
            })),
            original.claim.clone(),
        )?;
        if super::journal::classify_original_actor(&original.scope, &semantic)?
            != OriginalActor::Agent
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "recovery reference is not an original agent bootstrap",
            ));
        }
        self.request.identity.validate().map_err(io::Error::other)?;
        if self.request.operation
            != self
                .request
                .decision_operation()
                .map_err(io::Error::other)?
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "recovery assertion decision key mismatch",
            ));
        }
        Command::RecoverBootstrap(Box::new(self.request.clone()))
            .validate()
            .map_err(io::Error::other)
    }
}

fn original(
    journal: &Journal,
    reference: &str,
) -> Result<(IntentRef, BootstrapIdentity), RunError> {
    let pending = super::retry::load_original_for_actor(journal, reference)?;
    if super::journal::classify_original_actor(&pending.header.scope, &pending.semantic)?
        != OriginalActor::Agent
    {
        return Err(super::invalid_request(
            "recovery requires an original agent bootstrap",
        ));
    }
    let SemanticMutation::Frozen { claim, mutation } = pending.semantic else {
        return Err(super::invalid_request(
            "bootstrap needs frozen original caller",
        ));
    };
    let SemanticMutation::HandoffBootstrap(plan) = *mutation else {
        return Err(super::invalid_request(
            "recovery reference is not a bootstrap",
        ));
    };
    let identity = BootstrapIdentity {
        compound: plan.payload.handoff.keys.compound.clone(),
        scope: pending.header.scope,
        claim,
        digest: pending.header.semantic_digest,
        payload: plan.payload,
    };
    identity.validate().map_err(super::invalid_request)?;
    Ok((pending.header.reference, identity))
}

/// Freeze a request against the exact original. Production must supply real
/// fresh CreatedPane evidence and canonical cancellation guards at activation.
/// No current binding, snapshot or PID can replace the original or quiescence assertion.
pub fn prepare(
    journal: &Journal,
    request: &Request,
    disposition: BootstrapRecoveryDisposition,
    operator_uid: u32,
    namespace: &HandoffNamespace,
) -> Result<RecoveryPlan, RunError> {
    let (original_ref, identity) = original(journal, request.reference.as_str())?;
    if &identity.payload.handoff.namespace != namespace {
        return Err(super::invalid_request(
            "recovery namespace differs from original bootstrap",
        ));
    }
    let matches = match (&request.disposition, &disposition) {
        (
            Assertion::CreatedPane(pane),
            BootstrapRecoveryDisposition::CreatedPane { evidence, .. },
        ) => pane == &evidence.root_pane,
        (Assertion::NotCreated, BootstrapRecoveryDisposition::NotCreated { .. }) => true,
        (
            Assertion::Cancelled { reason },
            BootstrapRecoveryDisposition::Cancelled { reason: frozen, .. },
        ) => reason == frozen,
        _ => false,
    };
    if !matches {
        return Err(super::invalid_request(
            "recovery disposition differs from explicit assertion",
        ));
    }
    let mut request = RecoverBootstrap {
        identity,
        expected_attempt: request.attempt,
        operation: original_ref.operation.clone(),
        disposition,
    };
    request.operation = request
        .decision_operation()
        .map_err(super::invalid_request)?;
    let plan = RecoveryPlan {
        version: 1,
        original_ref,
        operator_uid,
        request,
    };
    plan.validate()?;
    Ok(plan)
}

fn validate_original(
    journal: &Journal,
    plan: &RecoveryPlan,
    namespace: &HandoffNamespace,
) -> Result<(), RunError> {
    plan.validate()?;
    let (reference, identity) = original(journal, &plan.original_ref.recovery_ref())?;
    if reference != plan.original_ref
        || identity != plan.request.identity
        || &identity.payload.handoff.namespace != namespace
    {
        return Err(super::invalid_request(
            "recovery original identity or namespace mismatch",
        ));
    }
    if plan.operator_uid != unsafe { libc::geteuid() } {
        return Err(super::invalid_request(
            "recovery belongs to a different local account",
        ));
    }
    Ok(())
}

/// Publish only the operator decision. Never modify the original intent.
pub fn publish(
    journal: &Journal,
    plan: &RecoveryPlan,
    namespace: &HandoffNamespace,
    created_at_millis: i64,
) -> Result<IntentRef, RunError> {
    // Refuse an invalid retained original before the lock can create a file.
    validate_original(journal, plan, namespace)?;
    let _lock = super::handoff::lock(journal, &plan.original_ref)?;
    // Recheck after exclusion to cover changes between validation and locking.
    validate_original(journal, plan, namespace)?;
    Ok(journal.record(
        IntentScope::Operator {
            instance: namespace.instance.clone(),
            local_user_uid: plan.operator_uid,
        },
        SemanticMutation::OperatorRecoverBootstrap(Box::new(plan.clone())),
        created_at_millis,
    )?)
}

/// Internal guarded-canonical consumer only. Public routes remain inert until
/// production actor/namespace/fresh observation guards are wired in task19.
/// Hold the original normal operation lock for the entire deciding call.
#[allow(clippy::too_many_arguments)]
pub(crate) fn retry_to_writer<C: crate::ports::LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    namespace: &HandoffNamespace,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<CommandResult, RunError> {
    let pending = journal.load(reference)?;
    if super::journal::classify_original_actor(&pending.header.scope, &pending.semantic)?
        != OriginalActor::HumanOrOperator
    {
        return Err(super::invalid_request(
            "recovery retry needs an operator decision",
        ));
    }
    let SemanticMutation::OperatorRecoverBootstrap(plan) = pending.semantic else {
        return Err(super::invalid_request("not an operator bootstrap recovery"));
    };
    // The operator origin alone does not validate the retained agent original.
    // Check it before the operation lock can create any local state.
    validate_original(journal, &plan, namespace)?;
    let _lock = super::handoff::lock(journal, &plan.original_ref)?;
    validate_original(journal, &plan, namespace)?;
    let result = client.call(
        Command::RecoverBootstrap(Box::new(plan.request.clone())),
        &super::cooperative_budget(clock),
    )?;
    let CommandResult::BootstrapRecovered(saved) = &result else {
        return Err(super::invalid_request("unexpected recovery result"));
    };
    if saved.identity != plan.request.identity
        || saved.attempt != plan.request.expected_attempt
        || saved.operation != plan.request.operation
        || saved.disposition != plan.request.disposition
        || saved.operator_uid != plan.operator_uid
        || saved.operator_provenance != format!("operator:local-user:{}", plan.operator_uid)
    {
        return Err(super::invalid_request(
            "recovery result differs from frozen operator decision",
        ));
    }
    super::output::write_selected(
        &result,
        output,
        crate::store::topology_handoff::MAX_RECOVERY_BYTES as u32 + 1024,
        writer,
    )?;
    journal.complete(reference)?;
    Ok(result)
}

/// Ready argv pins the original ref and inspected attempt, never latest attempt.
pub fn recovery_argv(prefix: &[String], request: &Request) -> Vec<String> {
    let mut argv = prefix.to_vec();
    argv.insert(1.min(argv.len()), "human".into());
    argv.extend([
        "handoff".into(),
        "recover".into(),
        request.reference.as_str().into(),
        "--attempt".into(),
        request.attempt.get().to_string(),
    ]);
    match &request.disposition {
        Assertion::CreatedPane(pane) => {
            argv.extend(["--created-pane".into(), pane.as_str().into()])
        }
        Assertion::NotCreated => argv.push("--not-created".into()),
        Assertion::Cancelled { reason } => {
            argv.extend(["--cancel".into(), "--reason".into(), reason.clone()])
        }
    }
    argv
}

#[cfg(test)]
#[path = "../../tests/cli/topology_recover.rs"]
mod tests;
