//! The CLI's site-capability gate (F9): before a request that carries a non-Pane caller site is
//! sent, the daemon must advertise the local capability that understands it. Without it the CLI
//! refuses with a fixed line instead of sending a request an older daemon would reject as an
//! unknown field. Named sites are gated on `seat.host_sites_v1` (M13); terminal sites
//! (`seat.terminal_sites_v1`) add theirs with the phase that builds the site. A Pane site never
//! goes through this gate.

use super::caller_location::CallerSite;
use crate::ports::LocalClient;
use crate::protocol::{
    capabilities::HOST_SITES_V1,
    commands::ResolveThreadQuery,
    ids::{HostTargetId, SeatId},
    results::{ApiError, ErrorCode},
    time::CallBudget,
};

/// M13 (63 bytes): the daemon does not advertise the capability a non-Pane site needs.
pub const M13: &str = "herdr-threads: daemon predates named/terminal seats; restart it";

/// Passes when the daemon advertises `capability`; otherwise refuses with `refusal_line` as an
/// `Unsupported` error, before anything is sent.
pub fn require_site_capability(
    client: &dyn LocalClient,
    capability: &str,
    refusal_line: &str,
    budget: &CallBudget,
) -> Result<(), ApiError> {
    if client.supports_capability(capability, budget) {
        Ok(())
    } else {
        Err(ApiError::new(ErrorCode::Unsupported, refusal_line))
    }
}

/// The thread-resolution request for the caller's site. A site that sends a host scope (Named)
/// first passes the `seat.host_sites_v1` gate (M13) and then carries its host and target; any
/// other caller keeps the pane target it was given and sends no host, so its bytes are unchanged.
pub fn resolve_thread_query(
    site: Option<CallerSite<'_>>,
    selector: &str,
    caller: Option<SeatId>,
    caller_target: Option<HostTargetId>,
    client: &dyn LocalClient,
    budget: &CallBudget,
) -> Result<ResolveThreadQuery, ApiError> {
    let scope = site.and_then(CallerSite::sent_scope);
    if scope.is_some() {
        require_site_capability(client, HOST_SITES_V1, M13, budget)?;
    }
    let (caller_target, caller_host) = match scope {
        Some((host, target)) => (Some(target), Some(host)),
        None => (caller_target, None),
    };
    Ok(ResolveThreadQuery {
        selector: selector.to_owned(),
        caller,
        caller_target,
        caller_host,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        commands::Command,
        results::CommandResult,
        time::{Cancellation, MonoInstant},
    };

    const TEST_CAP: &str = "test.site_v1";
    const LINE: &str = "herdr-threads: daemon predates named/terminal seats; restart it";

    struct Fake(&'static [&'static str]);
    impl LocalClient for Fake {
        fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
            self.0.contains(&name)
        }
        fn call(&self, _: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
            unreachable!("the gate never sends a command")
        }
        fn call_with_output(
            &self,
            _: Command,
            _: &crate::protocol::output::OutputSpec,
            _: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            unreachable!("the gate never sends a command")
        }
    }

    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        }
    }

    #[test]
    fn refuses_with_configured_line_without_capability() {
        let error =
            require_site_capability(&Fake(&["other.cap"]), TEST_CAP, LINE, &budget()).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(error.detail, LINE);
    }

    #[test]
    fn passes_with_capability() {
        require_site_capability(&Fake(&[TEST_CAP]), TEST_CAP, LINE, &budget()).unwrap();
    }
}
