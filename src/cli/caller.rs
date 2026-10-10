//! Who is calling (spec §1, decision 15): an agent caller's identity comes from the harness
//! environment, a person's from the pane's saved context.
//!
//! The process entrypoint ([`derive_from_process`]) switches derivation on; library and test
//! callers default to the legacy pane-context path unless a test fixes the origin
//! ([`fix_origin`]). Nothing here is adversarially verified (TRUST-POLICY: cooperative,
//! same-user).
use super::{RunError, commands::ParsedCli};
use crate::{
    harness::{
        caller_env::{self, EnvClaim, EnvRefusal},
        context::{Harness, OccupantContext, Role, SessionReference, session_execution},
    },
    protocol::{
        authority::CallerRole,
        ids::{HostTargetId, SeatId},
        results::ApiError,
    },
};
use std::cell::RefCell;

/// How this invocation identifies itself.
#[derive(Debug, Clone)]
pub enum CallerOrigin {
    /// No harness ancestor and no harness marker: a person. Pane-context attribution.
    Human,
    /// A harness session, from its environment.
    Agent(EnvClaim),
    /// An agent whose identity cannot be established: refused before any daemon call.
    Refused(EnvRefusal),
}

#[derive(Clone)]
enum Source {
    /// Library and test callers: the saved pane context decides, as before.
    Legacy,
    /// The process entrypoint: derive from `std::env` and the host process table.
    Process,
    #[cfg(any(test, feature = "test-support"))]
    Fixed(CallerOrigin),
}

thread_local! {
    static SOURCE: RefCell<Source> = const { RefCell::new(Source::Legacy) };
    static ACTIVE: RefCell<Option<CallerOrigin>> = const { RefCell::new(None) };
}

/// Called by the process entrypoint only: every cooperative command derives its caller from the
/// environment from here on.
pub fn derive_from_process() {
    SOURCE.with(|source| *source.borrow_mut() = Source::Process);
}

/// Test fixture: this thread's commands run as `origin`, as if derived from the environment.
#[cfg(any(test, feature = "test-support"))]
pub fn fix_origin(origin: CallerOrigin) -> OriginFixture {
    OriginFixture(SOURCE.with(|source| source.replace(Source::Fixed(origin))))
}
#[cfg(any(test, feature = "test-support"))]
pub struct OriginFixture(Source);
#[cfg(any(test, feature = "test-support"))]
impl Drop for OriginFixture {
    fn drop(&mut self) {
        SOURCE.with(|source| *source.borrow_mut() = self.0.clone());
    }
}

/// The origin of `parsed`, or `None` on the legacy path.
pub(super) fn derive(parsed: &ParsedCli) -> Option<CallerOrigin> {
    match SOURCE.with(|source| source.borrow().clone()) {
        Source::Legacy => None,
        #[cfg(any(test, feature = "test-support"))]
        Source::Fixed(origin) => Some(origin),
        Source::Process => Some(match caller_env::derive_current(parsed.agent_id.clone()) {
            Ok(claim) => CallerOrigin::Agent(claim),
            Err(EnvRefusal::NoHarness) if !super::output::harness_marked_now() => {
                CallerOrigin::Human
            }
            Err(refusal) => CallerOrigin::Refused(refusal),
        }),
    }
}

/// The origin in force for the command running on this thread.
pub(super) fn active() -> Option<CallerOrigin> {
    ACTIVE.with(|active| active.borrow().clone())
}

/// Keeps `origin` active for the command running on this thread until dropped.
pub(super) struct ActiveGuard(Option<CallerOrigin>);
impl ActiveGuard {
    pub(super) fn enter(origin: Option<CallerOrigin>) -> Self {
        Self(ACTIVE.with(|active| active.replace(origin)))
    }
}
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.with(|active| *active.borrow_mut() = self.0.take());
    }
}

/// The stable error for a refused origin (decision 15; exit statuses in `exit.rs`).
pub(super) fn refusal_error(refusal: &EnvRefusal) -> RunError {
    let api = match refusal {
        EnvRefusal::NoHarness => ApiError::session_claim_required(
            "an agent harness environment marker is set, but no claude, codex or hermes process \
             is among this process's ancestors, so the call cannot be attributed to a harness \
             session; run herdr-threads from the agent's own shell tool",
        ),
        EnvRefusal::MissingSession { .. } => ApiError::session_claim_required(refusal.message()),
        EnvRefusal::NoFingerprint { .. } => {
            ApiError::liveness_fingerprint_required(refusal.message())
        }
        EnvRefusal::ManagedDaemon { .. } => ApiError::managed_daemon_refused(refusal.message()),
    };
    RunError::Api(api)
}

pub(super) fn role_of(role: CallerRole) -> Role {
    match role {
        CallerRole::TopLevel => Role::TopLevel,
        CallerRole::Subagent => Role::Subagent,
    }
}

/// The agent session's context at the seat's mapping generation: derived, never saved. A
/// subagent's context is its parent session's (the journal holds top-level contexts only;
/// subagents never check in or write).
pub(super) fn agent_context(
    env: &EnvClaim,
    instance: uuid::Uuid,
    seat: &SeatId,
    target: &HostTargetId,
    generation: u64,
) -> OccupantContext {
    let harness = Harness::from(env.harness);
    OccupantContext {
        format_version: 1,
        instance,
        seat: seat.as_str().into(),
        target: target.as_str().into(),
        harness,
        binding_generation: generation,
        execution: session_execution(instance, harness, env.native_session.as_str(), None),
        session: SessionReference::Native(env.native_session.as_str().into()),
        role: Role::TopLevel,
    }
}

/// Refuse a selection (`--cooperative-*` flags or the pane's) whose harness is not the calling
/// agent's harness: the claim's harness is the caller's, never a flag's. A person's process
/// keeps the pane-context path (the daemon refuses an agent claim without a session).
pub(super) fn check_selection(origin: &CallerOrigin, harness: Harness) -> Result<(), RunError> {
    match origin {
        CallerOrigin::Agent(env) if Harness::from(env.harness) != harness => {
            Err(RunError::Api(ApiError::invalid_request(format!(
                "the selected harness {} differs from the calling harness {}",
                harness.as_str(),
                Harness::from(env.harness).as_str()
            ))))
        }
        _ => Ok(()),
    }
}
