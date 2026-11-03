//! The CLI's site-capability gate (F9): before a request that carries a non-Pane caller site is
//! sent, the daemon must advertise the local capability that understands it. Without it the CLI
//! refuses with a fixed line instead of sending a request an older daemon would reject as an
//! unknown field. Named sites are gated on `seat.host_sites_v1` and Terminal sites on
//! `seat.terminal_sites_v1` (M13), so a new CLI never sends a Terminal site to a daemon that only
//! knows Named ones. A Pane site never goes through this gate.

use super::caller_location::CallerSite;
use crate::ports::LocalClient;
use crate::protocol::{
    capabilities::SEAT_READONLY_V1,
    commands::{Command, ResolveThreadQuery},
    ids::{HostLocator, HostTargetId, SeatId},
    results::{ApiError, ErrorCode},
    time::CallBudget,
};

/// M13 (63 bytes): the daemon does not advertise the capability a non-Pane site needs.
pub const M13: &str = "herdr-threads: daemon predates named/terminal seats; restart it";

/// A refused readonly check-in (58 bytes): the daemon does not advertise `seat.readonly_v1`.
pub const READONLY_REFUSAL: &str = "herdr-threads: daemon predates read-only seats; restart it";

/// Passes a check-in without `readonly`, with no round trip, so a default caller's bytes are
/// unchanged. One that carries it goes only to a daemon that advertises [`SEAT_READONLY_V1`].
pub fn require_readonly<C: LocalClient + ?Sized>(
    command: &Command,
    client: &C,
    budget: &CallBudget,
) -> Result<(), ApiError> {
    match command {
        Command::CheckIn(check) | Command::OperatorCheckIn(check)
            if check.readonly && !client.supports_capability(SEAT_READONLY_V1, budget) =>
        {
            Err(ApiError::new(ErrorCode::Unsupported, READONLY_REFUSAL))
        }
        _ => Ok(()),
    }
}

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

/// Passes when the daemon advertises what `site` needs to be sent (its scope is sent only then);
/// a Pane site needs nothing.
pub fn require_site(
    site: CallerSite<'_>,
    client: &dyn LocalClient,
    budget: &CallBudget,
) -> Result<(), ApiError> {
    match site.required_capability() {
        Some(capability) => require_site_capability(client, capability, M13, budget),
        None => Ok(()),
    }
}

/// The host a seat lookup for `site` carries (`SeatsQuery.host`): a Named or Terminal site passes
/// its capability gate (M13) and sends its host locator (the reserved named host, the core's
/// socket); a Pane site sends none, so its bytes are unchanged.
pub fn seats_host(
    site: Option<CallerSite<'_>>,
    client: &dyn LocalClient,
    budget: &CallBudget,
) -> Result<Option<HostLocator>, ApiError> {
    let Some(site) = site else {
        return Ok(None);
    };
    let Some((host, _)) = site.sent_scope() else {
        return Ok(None);
    };
    require_site(site, client, budget)?;
    Ok(Some(host))
}

/// The thread-resolution request for the caller's site. A site that sends a host scope (Named,
/// Terminal) first passes its capability gate (M13) and then carries its host and target; any
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
    if let (Some(site), Some(_)) = (site, &scope) {
        require_site(site, client, budget)?;
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

    use crate::protocol::capabilities::{HOST_SITES_V1, TERMINAL_SITES_V1};

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

    fn check_in(readonly: bool) -> crate::protocol::commands::CheckIn {
        use crate::protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            commands::{CheckIn, CheckInMode},
            ids::{ExecutionId, HostTargetId, NativeSessionId, OperationId},
        };
        CheckIn {
            mode: CheckInMode::Current,
            claim: CallerClaim {
                instance: "i".into(),
                host: HostLocator::parse("/claim.sock").unwrap(),
                seat: SeatId::new("s"),
                binding_generation: 1,
                role: CallerRole::TopLevel,
                harness: Harness::Codex,
                native_session: NativeSessionId::new("n"),
                execution: ExecutionId::new("e"),
                target: HostTargetId::new("w1:p1"),
                agent_id: None,
                liveness: None,
            },
            operation: OperationId::new("op"),
            lineage_parent: None,
            readonly,
        }
    }

    #[test]
    fn readonly_check_in_needs_the_capability() {
        let command = Command::CheckIn(check_in(true));
        let error = require_readonly(&command, &Fake(&[]), &budget()).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(error.detail, READONLY_REFUSAL);
        require_readonly(&command, &Fake(&[SEAT_READONLY_V1]), &budget()).unwrap();
    }

    #[test]
    fn operator_check_in_is_gated_alike() {
        let command = Command::OperatorCheckIn(check_in(true));
        let error = require_readonly(&command, &Fake(&[]), &budget()).unwrap_err();
        assert_eq!(error.detail, READONLY_REFUSAL);
        require_readonly(&command, &Fake(&[SEAT_READONLY_V1]), &budget()).unwrap();
    }

    struct Panics;
    impl LocalClient for Panics {
        fn supports_capability(&self, _: &str, _: &CallBudget) -> bool {
            panic!("a plain check-in asks the daemon nothing")
        }
        fn call(&self, _: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
            unreachable!()
        }
        fn call_with_output(
            &self,
            _: Command,
            _: &crate::protocol::output::OutputSpec,
            _: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            unreachable!()
        }
    }

    #[test]
    fn plain_check_in_asks_nothing() {
        require_readonly(&Command::CheckIn(check_in(false)), &Panics, &budget()).unwrap();
        require_readonly(
            &Command::OperatorCheckIn(check_in(false)),
            &Panics,
            &budget(),
        )
        .unwrap();
        assert_eq!(READONLY_REFUSAL.len(), 58);
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

    #[test]
    fn named_seat_lookup_sends_the_named_host_behind_the_gate() {
        use super::super::caller_location::CallerLocator;
        use crate::protocol::ids::{NAMED_HOST_LOCATOR, SeatName};
        let named = CallerLocator::Named(SeatName::parse("alice").unwrap());
        let host = seats_host(Some(named.site()), &Fake(&[HOST_SITES_V1]), &budget())
            .unwrap()
            .unwrap();
        assert_eq!(host.as_str(), NAMED_HOST_LOCATOR);
        let error = seats_host(Some(named.site()), &Fake(&[]), &budget()).unwrap_err();
        assert_eq!(error.detail, M13);
        let pane = CallerLocator::Pane { id: "p_1".into() };
        assert!(
            seats_host(Some(pane.site()), &Fake(&[]), &budget())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn terminal_site_needs_its_own_capability_not_the_named_one() {
        use super::super::caller_location::CallerLocator;
        let terminal = CallerLocator::Terminal {
            socket: "/core/work.sock".into(),
            id: "t2".into(),
        };
        let host = seats_host(
            Some(terminal.site()),
            &Fake(&[HOST_SITES_V1, TERMINAL_SITES_V1]),
            &budget(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(host.as_str(), "/core/work.sock");
        // A daemon that only knows Named sites refuses with M13 and nothing is sent.
        let error =
            seats_host(Some(terminal.site()), &Fake(&[HOST_SITES_V1]), &budget()).unwrap_err();
        assert_eq!(error.detail, M13);
        let query = resolve_thread_query(
            Some(terminal.site()),
            "ops",
            None,
            None,
            &Fake(&[TERMINAL_SITES_V1]),
            &budget(),
        )
        .unwrap();
        assert_eq!(query.caller_host.unwrap().as_str(), "/core/work.sock");
        assert_eq!(query.caller_target.unwrap().as_str(), "t2");
    }
}
