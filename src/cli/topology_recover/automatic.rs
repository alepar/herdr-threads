//! Exact, durable operator decisions for a frozen automatic bootstrap parent.
use super::{Assertion, Request};
use crate::{
    cli::{
        RunError,
        actor_route::InvocationActor,
        journal::{IntentRef, IntentScope, Journal, OriginalActor, SemanticMutation},
    },
    ports::LocalClient,
    protocol::{
        commands::Command,
        handoff::{
            AutomaticBootstrapAction, AutomaticBootstrapIdentity, AutomaticBootstrapMutation,
            AutomaticBootstrapRecoveryDisposition, AutomaticBootstrapResult,
            BootstrapQuiescenceAssertion, HandoffNamespace,
        },
        output::OutputSpec,
        results::CommandResult,
        time::Clock,
    },
};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPlan {
    pub version: u32,
    pub original_ref: IntentRef,
    pub operator_uid: u32,
    pub request: AutomaticBootstrapMutation,
}

impl RecoveryPlan {
    pub fn validate(&self) -> io::Result<()> {
        if self.version != 2 || self.original_ref.ordinal == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid automatic recovery plan",
            ));
        }
        self.request.validate().map_err(io::Error::other)?;
        if !matches!(
            self.request.action,
            AutomaticBootstrapAction::Recover { .. }
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "automatic operator decision needs recovery action",
            ));
        }
        self.request.operation().map_err(io::Error::other)?;
        Ok(())
    }
}

fn original(
    journal: &Journal,
    reference: &str,
) -> Result<(IntentRef, AutomaticBootstrapIdentity), RunError> {
    let pending = crate::cli::retry::load_original_for_actor(journal, reference)?;
    if crate::cli::journal::classify_original_actor(&pending.header.scope, &pending.semantic)?
        != OriginalActor::Agent
    {
        return Err(crate::cli::invalid_request(
            "recovery requires original agent automatic bootstrap",
        ));
    }
    let identity = crate::cli::topology_handoff::automatic_bootstrap_identity(&pending)?;
    Ok((pending.header.reference, identity))
}

fn exact_namespace(id: &AutomaticBootstrapIdentity, ns: &HandoffNamespace) -> bool {
    let frozen = &id.plan.payload.handoff.namespace;
    frozen.instance == ns.instance
        && frozen.state_dir.as_os_str() == ns.state_dir.as_os_str()
        && frozen.host_endpoint.as_os_str() == ns.host_endpoint.as_os_str()
}

pub(crate) fn require_capability<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    crate::cli::topology_runtime::require_automatic_capability(client, clock)
}

pub(crate) fn prepare(
    journal: &Journal,
    request: &Request,
    disposition: AutomaticBootstrapRecoveryDisposition,
    operator_uid: u32,
    namespace: &HandoffNamespace,
    inspected: &AutomaticBootstrapResult,
) -> Result<RecoveryPlan, RunError> {
    let (original_ref, identity) = original(journal, request.reference.as_str())?;
    if !exact_namespace(&identity, namespace) {
        return Err(crate::cli::invalid_request(
            "automatic recovery namespace differs from original",
        ));
    }
    let matches_assertion = match (&request.disposition, &disposition) {
        (
            Assertion::CreatedPane(target),
            AutomaticBootstrapRecoveryDisposition::CreatedPane { evidence, .. },
        ) => target == &evidence.root_pane,
        (Assertion::NotCreated, AutomaticBootstrapRecoveryDisposition::NotCreated { .. }) => true,
        (
            Assertion::Cancelled { reason },
            AutomaticBootstrapRecoveryDisposition::Cancelled { reason: frozen, .. },
        ) => reason == frozen,
        _ => false,
    };
    if !matches_assertion || inspected.identity != identity || inspected.attempt != request.attempt
    {
        return Err(crate::cli::invalid_request(
            "automatic recovery differs from inspected attempt or assertion",
        ));
    }
    let plan = RecoveryPlan {
        version: 2,
        original_ref,
        operator_uid,
        request: AutomaticBootstrapMutation {
            identity,
            action: AutomaticBootstrapAction::Recover {
                attempt: request.attempt,
                inspection: inspected
                    .recovery_inspection()
                    .map_err(crate::cli::invalid_request)?,
                disposition,
            },
        },
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
        || !exact_namespace(&identity, namespace)
        || plan.operator_uid != unsafe { libc::geteuid() }
    {
        return Err(crate::cli::invalid_request(
            "automatic operator decision original, namespace or local user differs",
        ));
    }
    Ok(())
}

pub(crate) fn publish_under_guard(
    journal: &Journal,
    plan: &RecoveryPlan,
    namespace: &HandoffNamespace,
    created_at_millis: i64,
    _guard: &std::fs::File,
) -> Result<IntentRef, RunError> {
    validate_original(journal, plan, namespace)?;
    Ok(journal.record(
        IntentScope::Operator {
            instance: namespace.instance.clone(),
            local_user_uid: plan.operator_uid,
        },
        SemanticMutation::OperatorRecoverAutomaticBootstrap(Box::new(plan.clone())),
        created_at_millis,
    )?)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn retry_to_writer<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    namespace: &HandoffNamespace,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<CommandResult, RunError> {
    let pending = crate::cli::retry::load_original_for_actor(journal, &reference.recovery_ref())?;
    if crate::cli::journal::classify_original_actor(&pending.header.scope, &pending.semantic)?
        != OriginalActor::HumanOrOperator
    {
        return Err(crate::cli::invalid_request(
            "automatic recovery retry needs operator origin",
        ));
    }
    let SemanticMutation::OperatorRecoverAutomaticBootstrap(plan) = pending.semantic else {
        return Err(crate::cli::invalid_request(
            "not an automatic operator decision",
        ));
    };
    validate_original(journal, &plan, namespace)?;
    require_capability(client, clock)?;
    let guard = crate::cli::handoff::lock(journal, &plan.original_ref)?;
    retry_under_guard(
        journal, reference, namespace, client, clock, output, writer, &guard,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn retry_under_guard<C: LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    namespace: &HandoffNamespace,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
    _guard: &std::fs::File,
) -> Result<CommandResult, RunError> {
    if crate::cli::retry::preflight_original_actor(
        journal.root(),
        &reference.recovery_ref(),
        InvocationActor::Human,
        &output.context,
    )? != OriginalActor::HumanOrOperator
    {
        return Err(crate::cli::invalid_request(
            "automatic recovery retry needs operator decision",
        ));
    }
    let pending = journal.load(reference)?;
    let SemanticMutation::OperatorRecoverAutomaticBootstrap(plan) = pending.semantic else {
        return Err(crate::cli::invalid_request(
            "not an automatic operator decision",
        ));
    };
    validate_original(journal, &plan, namespace)?;
    require_capability(client, clock)?;
    let result = client.call(
        Command::AutomaticBootstrap(Box::new(plan.request.clone())),
        &crate::cli::cooperative_budget(clock),
    )?;
    let CommandResult::AutomaticBootstrap(saved) = &result else {
        return Err(crate::cli::invalid_request(
            "unexpected automatic recovery result",
        ));
    };
    let AutomaticBootstrapAction::Recover {
        attempt,
        inspection,
        disposition,
    } = &plan.request.action
    else {
        return Err(crate::cli::invalid_request("invalid recovery action"));
    };
    let recovered = saved.recovery.as_ref().ok_or_else(|| {
        crate::cli::invalid_request("automatic recovery result lacks operator decision")
    })?;
    if saved.identity != plan.request.identity
        || recovered.operation
            != plan
                .request
                .operation()
                .map_err(crate::cli::invalid_request)?
        || &recovered.attempt != attempt
        || &recovered.inspection != inspection
        || &recovered.disposition != disposition
        || recovered.operator_uid != plan.operator_uid
        || recovered.operator_provenance != format!("operator:local-user:{}", plan.operator_uid)
    {
        return Err(crate::cli::invalid_request(
            "automatic recovery result differs from frozen operator decision",
        ));
    }
    // A live parent can carry the identity, the exact derived child and a
    // cancellation decision retaining that child again.
    crate::cli::output::write_selected(
        &result,
        output,
        4 * crate::store::topology_handoff::MAX_IDENTITY_BYTES as u32 + 4096,
        writer,
    )?;
    journal.complete(reference)?;
    Ok(result)
}

pub(crate) fn not_created() -> AutomaticBootstrapRecoveryDisposition {
    AutomaticBootstrapRecoveryDisposition::NotCreated {
        quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
    }
}

pub(crate) fn cancelled(
    reason: String,
    inspected: &AutomaticBootstrapResult,
) -> AutomaticBootstrapRecoveryDisposition {
    AutomaticBootstrapRecoveryDisposition::Cancelled {
        reason,
        quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
        attached_child: inspected
            .attachment
            .as_ref()
            .map(|v| Box::new(v.child.clone())),
    }
}
