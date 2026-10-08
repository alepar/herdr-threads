//! Retry coordinator: fresh invocation evidence, replay, and output completion.
use crate::{
    cli::journal::{IntentRef, IntentScope, Journal, SemanticMutation},
    protocol::{
        authority::CallerClaim,
        commands::Command,
        output::OutputSpec,
        results::{ApiError, CommandResult, ErrorCode},
    },
};
use std::io::{self, Write};

/// Administrative recovery retry classifies the separate operator origin before
/// any lock, output or cleanup. Production activation supplies canonical guards.
#[allow(clippy::too_many_arguments)]
pub fn run_topology_recovery_retry_to_writer<C: crate::ports::LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    actor: super::actor_route::InvocationActor,
    namespace: &crate::protocol::handoff::HandoffNamespace,
    client: &C,
    clock: &dyn crate::protocol::time::Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<CommandResult, super::RunError> {
    if preflight_original_actor(
        journal.root(),
        &reference.recovery_ref(),
        actor,
        &output.context,
    )? != super::journal::OriginalActor::HumanOrOperator
    {
        return Err(super::invalid_request(
            "recovery retry requires original operator decision",
        ));
    }
    super::topology_recover::retry_to_writer(
        journal, reference, namespace, client, clock, output, writer,
    )
}

/// Additive internal delivery consumer. Public dispatch stays Unsupported until
/// activation supplies canonical namespace/current-recipient guards. The wrapper
/// classifies exact immutable original bytes before entering the strict executor.
#[allow(clippy::too_many_arguments)]
pub fn run_delivery_retry_to_writer<C: crate::ports::LocalClient + ?Sized, W: Write>(
    journal: &Journal,
    reference: &IntentRef,
    actor: super::actor_route::InvocationActor,
    namespace: &crate::protocol::handoff::HandoffNamespace,
    client: &C,
    clock: &dyn crate::protocol::time::Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<serde_json::Value, super::RunError> {
    preflight_original_actor(
        journal.root(),
        &reference.recovery_ref(),
        actor,
        &output.context,
    )?;
    let selected =
        super::handoff_delivery::resolve_recovery_ref(journal, &reference.recovery_ref())?;
    if selected != *reference {
        return Err(super::invalid_request(
            "delivery original reference mismatch",
        ));
    }
    super::handoff_delivery::retry_to_writer(
        journal, reference, namespace, client, clock, output, writer,
    )
}

/// Additive internal bootstrap consumer; public routing remains inert.
#[allow(clippy::too_many_arguments)]
pub fn run_bootstrap_retry<
    C: crate::ports::LocalClient + ?Sized,
    N: crate::ports::CreateTabPort + ?Sized,
>(
    journal: &Journal,
    reference: &IntentRef,
    actor: super::actor_route::InvocationActor,
    namespace: &crate::protocol::handoff::HandoffNamespace,
    client: &C,
    native: &N,
    clock: &dyn crate::protocol::time::Clock,
    submission: super::topology_handoff::BootstrapSubmissionInputs<'_>,
) -> Result<crate::protocol::handoff::BootstrapResult, super::RunError> {
    let context = crate::protocol::output::ContinuationContext {
        state_dir: Some(namespace.state_dir.to_string_lossy().into_owned()),
        host: Some(namespace.host_endpoint.to_string_lossy().into_owned()),
    };
    preflight_original_actor(journal.root(), &reference.recovery_ref(), actor, &context)?;
    let original = load_original_for_actor(journal, &reference.recovery_ref())?;
    if original.header.reference != *reference {
        return Err(super::invalid_request(
            "bootstrap original reference mismatch",
        ));
    }
    super::topology_handoff::resume_to_attachment(
        journal, reference, namespace, client, native, clock, submission,
    )
}

/// The command runner owns the real client and output writer. A journal error
/// returns before `submit`; every other error leaves the entry recoverable.
pub fn run_new<P, S, O>(
    journal: &Journal,
    scope: IntentScope,
    semantic: SemanticMutation,
    created_at_millis: i64,
    proof: P,
    submit: S,
    output: O,
) -> io::Result<(IntentRef, CommandResult)>
where
    P: FnOnce() -> io::Result<CallerClaim>,
    S: FnOnce(Command) -> io::Result<CommandResult>,
    O: FnOnce(&CommandResult) -> io::Result<()>,
{
    let reference = journal.record(scope.clone(), semantic, created_at_millis)?;
    let result = run_retry(journal, &reference, &scope, proof, submit, output)?;
    Ok((reference, result))
}

pub fn run_retry<P, S, O>(
    journal: &Journal,
    reference: &IntentRef,
    scope: &IntentScope,
    proof: P,
    submit: S,
    output: O,
) -> io::Result<CommandResult>
where
    P: FnOnce() -> io::Result<CallerClaim>,
    S: FnOnce(Command) -> io::Result<CommandResult>,
    O: FnOnce(&CommandResult) -> io::Result<()>,
{
    run_retry_inner(journal, reference, scope, proof, submit, output).map_err(|failure| {
        match failure {
            RetryFailure::Local(error) | RetryFailure::Submit(error) => error,
        }
    })
}

#[derive(Debug)]
pub enum RetryFailure<E> {
    Local(io::Error),
    Submit(E),
}

/// Publish the private intent before the first typed API submission.
// Allowed: journal, intent, proof, submit and output are independent inputs of one durable retry step.
#[allow(clippy::too_many_arguments)]
pub fn run_new_api_to_writer<P, S, W>(
    journal: &Journal,
    scope: IntentScope,
    semantic: SemanticMutation,
    created_at_millis: i64,
    proof: P,
    submit: S,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(IntentRef, CommandResult), RetryFailure<ApiError>>
where
    P: FnOnce() -> io::Result<CallerClaim>,
    S: FnOnce(Command) -> Result<CommandResult, ApiError>,
    W: Write,
{
    let reference = journal
        .record(scope.clone(), semantic, created_at_millis)
        .map_err(RetryFailure::Local)?;
    let result =
        run_retry_api_to_writer(journal, &reference, &scope, proof, submit, output, writer)?;
    Ok((reference, result))
}

/// A first submission of a fresh operation key whose daemon answer is a
/// definitive correlated rejection discards its intent. For cooperative
/// mutations only a deterministic code (see `is_deterministic_rejection`,
/// which records the decision for every request-shaped code) counts: the
/// request itself is invalid or refused and would be refused identically on
/// retry, while transient codes (`StoreBusy`,
/// `DeadlineExceeded`, `Cancelled`), `StoreCorrupt` and any unlisted code keep
/// the intent — a rejection does not prove nothing was committed under the key
/// (send preparation commits quanta before a later rejection). Seat resolution
/// (`ServiceAllocation`) commits nothing before deciding, so any correlated
/// rejection other than `UnknownOutcome` is definitive there. `UnknownOutcome`,
/// pre-submission failures and local output/journal failures always keep the
/// intent (CLI design "Durable client intent"). Retries of an existing intent
/// never discard it.
// Allowed: same inputs as run_new_api_to_writer; this is its rejection-discarding variant.
#[allow(clippy::too_many_arguments)]
pub fn run_new_api_to_writer_discarding_rejection<P, S, W>(
    journal: &Journal,
    scope: IntentScope,
    semantic: SemanticMutation,
    created_at_millis: i64,
    proof: P,
    submit: S,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(IntentRef, CommandResult), RetryFailure<ApiError>>
where
    P: FnOnce() -> io::Result<CallerClaim>,
    S: FnOnce(Command) -> Result<Result<CommandResult, ApiError>, ApiError>,
    W: Write,
{
    let cooperative = matches!(scope, IntentScope::Cooperative { .. });
    let definitive = move |code: &ErrorCode| {
        if cooperative {
            is_deterministic_rejection(code)
        } else {
            *code != ErrorCode::UnknownOutcome
        }
    };
    let reference = journal
        .record(scope.clone(), semantic, created_at_millis)
        .map_err(RetryFailure::Local)?;
    let result = run_retry_inner(
        journal,
        &reference,
        &scope,
        proof,
        |command| match submit(command) {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(rejection)) if definitive(&rejection.code) => {
                Err(Submitted::Rejected(rejection))
            }
            Ok(Err(uncertain)) | Err(uncertain) => Err(Submitted::Kept(uncertain)),
        },
        |result| {
            let bytes = super::output::emitted_bytes(result, output)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.detail))?;
            writer.write_all(&bytes)?;
            writer.flush()
        },
    );
    match result {
        Ok(result) => Ok((reference, result)),
        Err(RetryFailure::Local(error)) => Err(RetryFailure::Local(error)),
        Err(RetryFailure::Submit(Submitted::Kept(error))) => Err(RetryFailure::Submit(error)),
        Err(RetryFailure::Submit(Submitted::Rejected(rejection))) => {
            // A failed removal leaves an inert, still-retryable entry; the
            // daemon's typed rejection remains the reported outcome.
            let _ = journal.complete(&reference);
            Err(RetryFailure::Submit(rejection))
        }
    }
}

/// Codes a cooperative mutation answers from the frozen request plus committed
/// durable state, inside the mutation's own transaction and before anything is
/// committed under the operation key, so an identical retry with the same key
/// is refused identically unless another operation changes durable state
/// first. A transient, host, timing, budget or daemon-build condition does not
/// qualify, and a code no cooperative mutation can reach stays unlisted.
///
/// Request-shaped codes left out:
/// - `Unsupported` is not answered to a cooperative mutation by this daemon
///   build; an older build lacking the route answers it, and `retry` succeeds
///   after an upgrade.
/// - `UnsupportedHarness` is emitted only by the scheduler and client-side
///   setup/launch.
/// - `ThreadNotOrphaned` is operator orphan invite only.
/// - `RequiredInvitationNeedsManagedThread` is service-operation invite only.
///
/// Every other unlisted code keeps the intent (keep-on-doubt).
fn is_deterministic_rejection(code: &ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::InvalidRequest
            | ErrorCode::Unauthorized
            | ErrorCode::Archived
            | ErrorCode::Conflict
            | ErrorCode::OperationPayloadMismatch
            | ErrorCode::MembershipRequired
            | ErrorCode::NotFound
            | ErrorCode::StaleRequirementAcceptance
    )
}

/// A continuity refusal that depends on durable state and identical retry
/// would repeat (no match, several matches, owned target, an unusable
/// target). Transient, store, host, uncertain and not-yet-reconciled codes
/// (`ServiceBusy`, `StaleHostObservation`: a fresh observation or a finished
/// reconciliation can succeed) keep the intent, which the next resume in the
/// pane or `herdr-threads retry` replays under the same operation key.
pub(crate) fn is_continuity_refusal(code: &ErrorCode) -> bool {
    is_deterministic_rejection(code)
        || matches!(
            code,
            ErrorCode::NotFound
                | ErrorCode::TargetAlreadyOwned
                | ErrorCode::TargetUnresolved
                | ErrorCode::TargetUnsafe
                | ErrorCode::CallerUnverified
                | ErrorCode::Unsupported
                | ErrorCode::SequenceExhausted
        )
}

enum Submitted {
    Rejected(ApiError),
    Kept(ApiError),
}

/// Preserve the server's typed rejection or transport uncertainty for the
/// command runner. Neither branch retries implicitly.
pub fn run_retry_api_to_writer<P, S, W>(
    journal: &Journal,
    reference: &IntentRef,
    scope: &IntentScope,
    proof: P,
    submit: S,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<CommandResult, RetryFailure<ApiError>>
where
    P: FnOnce() -> io::Result<CallerClaim>,
    S: FnOnce(Command) -> Result<CommandResult, ApiError>,
    W: Write,
{
    run_retry_inner(journal, reference, scope, proof, submit, |result| {
        let bytes = super::output::emitted_bytes(result, output)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.detail))?;
        writer.write_all(&bytes)?;
        writer.flush()
    })
}

fn run_retry_inner<P, S, O, E>(
    journal: &Journal,
    reference: &IntentRef,
    scope: &IntentScope,
    proof: P,
    submit: S,
    output: O,
) -> Result<CommandResult, RetryFailure<E>>
where
    P: FnOnce() -> io::Result<CallerClaim>,
    S: FnOnce(Command) -> Result<CommandResult, E>,
    O: FnOnce(&CommandResult) -> io::Result<()>,
{
    let pending = journal.load(reference).map_err(RetryFailure::Local)?;
    if &pending.header.scope != scope {
        return Err(RetryFailure::Local(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "intent authority scope mismatch",
        )));
    }
    if let IntentScope::Operator { local_user_uid, .. } = scope
        && Some(*local_user_uid) != effective_uid()
    {
        return Err(RetryFailure::Local(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "operator intent belongs to a different effective user",
        )));
    }
    let claim = match scope {
        IntentScope::Cooperative { .. } => pending.semantic.frozen_claim().cloned(),
        IntentScope::Native { .. } => Some(proof().map_err(RetryFailure::Local)?),
        IntentScope::Operator { .. }
        | IntentScope::ServiceAllocation { .. }
        | IntentScope::Continuity { .. } => None,
    };
    let command = pending
        .semantic
        .to_command(reference.operation.clone(), claim)
        .map_err(RetryFailure::Local)?;
    let result = submit(command).map_err(RetryFailure::Submit)?;
    if !matches_result(&pending.semantic, &result) {
        return Err(RetryFailure::Local(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected mutation result",
        )));
    }
    output(&result).map_err(RetryFailure::Local)?;
    journal.complete(reference).map_err(RetryFailure::Local)?;
    Ok(result)
}

/// Complete the journal entry only after the selected bytes reach and flush
/// through the caller's writer. A write or flush error leaves it recoverable.
pub fn run_retry_to_writer<P, S, W>(
    journal: &Journal,
    reference: &IntentRef,
    scope: &IntentScope,
    proof: P,
    submit: S,
    output: &OutputSpec,
    writer: &mut W,
) -> io::Result<CommandResult>
where
    P: FnOnce() -> io::Result<CallerClaim>,
    S: FnOnce(Command) -> io::Result<CommandResult>,
    W: Write,
{
    run_retry(journal, reference, scope, proof, submit, |result| {
        let bytes = super::output::emitted_bytes(result, output)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.detail))?;
        writer.write_all(&bytes)?;
        writer.flush()
    })
}

#[cfg(unix)]
fn effective_uid() -> Option<u32> {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    Some(unsafe { geteuid() })
}
#[cfg(not(unix))]
fn effective_uid() -> Option<u32> {
    None
}

fn matches_result(request: &SemanticMutation, result: &CommandResult) -> bool {
    if let SemanticMutation::Frozen { mutation, .. } = request {
        return matches_result(mutation, result);
    }

    matches!(
        (request, result),
        (
            SemanticMutation::ResolveSeat { .. },
            CommandResult::SeatResolved(_)
        ) | (
            SemanticMutation::ContinuityCheckIn { .. },
            CommandResult::ContinuityReattached(_)
        ) | (
            SemanticMutation::CheckIn | SemanticMutation::CooperativeCheckIn { .. },
            CommandResult::CheckedIn(_)
        ) | (
            SemanticMutation::CreateThread { .. },
            CommandResult::ThreadCreated(_)
        ) | (
            SemanticMutation::Invite { .. },
            // An invite for a seat that already joined is a settled no-op
            // (ht-4is.3.12): no invitation episode, the intent completes.
            CommandResult::Invitation(_) | CommandResult::AlreadyJoined(_)
        ) | (SemanticMutation::Accept { .. }, CommandResult::Accepted(_))
            | (
                SemanticMutation::AcceptRequired { .. },
                CommandResult::RequiredAccepted(_)
            )
            | (
                SemanticMutation::SendMessage { .. },
                CommandResult::MessageSent(_)
            )
            | (
                SemanticMutation::Ack { .. } | SemanticMutation::AckDisplayed { .. },
                CommandResult::Acknowledged(_)
            )
            | (SemanticMutation::Reject { .. }, CommandResult::Rejected(_))
            | (SemanticMutation::Leave { .. }, CommandResult::Left(_))
            | (
                SemanticMutation::SetTopic { .. },
                CommandResult::TopicChanged(_)
            )
            | (
                SemanticMutation::SetThreadName { .. },
                CommandResult::ThreadNameChanged(_)
            )
            | (SemanticMutation::Archive { .. }, CommandResult::Archived(_))
            | (SemanticMutation::Reopen { .. }, CommandResult::Reopened(_))
            | (
                SemanticMutation::OperatorRebind { .. },
                CommandResult::OperatorRebound(_)
            )
            | (
                SemanticMutation::OperatorRetire { .. },
                CommandResult::OperatorRetired(_)
            )
            | (
                SemanticMutation::OperatorReplace { .. },
                CommandResult::OperatorRebound(_)
            )
            | (
                SemanticMutation::OperatorFreshSeat { .. },
                CommandResult::OperatorFreshSeat(_)
            )
            | (
                SemanticMutation::OperatorOrphanInvite { .. },
                CommandResult::OperatorInvited(_)
            )
    )
}

/// Read the immutable origin before any service, selection or completion effects.
pub fn preflight_original_actor(
    root: impl AsRef<std::path::Path>,
    recovery: &str,
    actor: super::actor_route::InvocationActor,
    context: &crate::protocol::output::ContinuationContext,
) -> io::Result<super::journal::OriginalActor> {
    let journal = Journal::read_only(root)?;
    let pending = load_original_for_actor(&journal, recovery)?;
    let original =
        super::journal::classify_original_actor(&pending.header.scope, &pending.semantic)?;
    if original == super::journal::OriginalActor::HumanOrOperator
        && actor == super::actor_route::InvocationActor::Agent
    {
        let mut argv = super::hook::cli_prefix(context);
        argv.insert(1, "human".into());
        argv.extend(["retry".into(), recovery.into()]);
        let command = argv
            .iter()
            .map(|token| {
                shlex::try_quote(token)
                    .map(|quoted| quoted.into_owned())
                    .map_err(io::Error::other)
            })
            .collect::<io::Result<Vec<_>>>()?
            .join(" ");
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("person/operator retry requires immediate human namespace; use {command}"),
        ));
    }
    Ok(original)
}

// Ordinary intents and retained delivery terminals share the exact local ordinal.
// Inspect both namespaces before classifying; never let a second origin disappear
// behind Journal's historical first-match lookup or an absent original intent.
pub(crate) fn load_original_for_actor(
    journal: &Journal,
    recovery: &str,
) -> io::Result<super::journal::PendingIntent> {
    crate::protocol::ids::LocalRecoveryRef::parse(recovery).map_err(io::Error::other)?;
    let ordinal: u64 = recovery
        .strip_prefix("local:")
        .unwrap()
        .parse()
        .map_err(io::Error::other)?;
    if ordinal == 0 || recovery != format!("local:{ordinal}") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "noncanonical intent reference",
        ));
    }
    let prefix = format!("{ordinal:020}-");
    let terminal_prefix = format!("delivery-{ordinal:020}-");
    let mut intents = 0;
    let mut retained = false;
    for entry in std::fs::read_dir(journal.root())? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".intent") {
            intents += 1;
        }
        retained |= name.starts_with(&terminal_prefix) && name.ends_with(".terminal");
    }
    if intents > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ambiguous original intent reference",
        ));
    }
    let pending = match journal.resolve_recovery_ref(recovery) {
        Ok(reference) => Some(journal.load(&reference)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound && retained => None,
        Err(error) => return Err(error),
    };
    // Validate scope/semantic and unfrozen handoff refusal before delivery's
    // stricter retained-origin decoder (which deliberately requires a claim).
    if let Some(pending) = &pending {
        super::journal::classify_original_actor(&pending.header.scope, &pending.semantic)?;
    }
    if retained
        && pending
            .as_ref()
            .is_some_and(|p| p.header.kind != crate::protocol::results::IntentKind::HandoffDelivery)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "conflicting original intent and delivery terminal",
        ));
    }
    if retained
        || pending
            .as_ref()
            .is_some_and(|p| p.header.kind == crate::protocol::results::IntentKind::HandoffDelivery)
    {
        let reference = super::handoff_delivery::resolve_recovery_ref(journal, recovery)
            .map_err(io::Error::other)?;
        super::handoff_delivery::load_original(journal, &reference).map_err(io::Error::other)
    } else {
        pending.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "original intent not found"))
    }
}
