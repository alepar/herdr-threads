//! The CLI's site-capability gate (F9): before a request that carries a non-Pane caller site is
//! sent, the daemon must advertise the local capability that understands it. Without it the CLI
//! refuses with a fixed line instead of sending a request an older daemon would reject as an
//! unknown field. Named sites are gated on `seat.host_sites_v1` and Terminal sites on
//! `seat.terminal_sites_v1` (M13), so a new CLI never sends a Terminal site to a daemon that only
//! knows Named ones. A Pane site never goes through this gate.

use super::caller_location::{CallerEnv, CallerSite, caller_continuity_key};
use super::{caller, commands::ParsedCli};
use crate::ports::LocalClient;
use crate::protocol::{
    capabilities::{SEAT_CONTINUITY_KEY_V1, SEAT_READONLY_V1},
    commands::{Command, ResolveThreadQuery},
    ids::{ContinuityKey, HostLocator, HostTargetId, SeatId},
    results::{ApiError, CommandResult, ErrorCode},
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

/// A refused keyed resolve (80 bytes): the key capability could not be read, so nothing was
/// journaled or sent; the next call reads again.
pub(crate) const KEY_CAPABILITY_UNREAD: &str =
    "continuity key capability (seat.continuity_key_v1) unread; retry; no seat minted";

/// What a caller may present to this daemon. `Unknown` is a key whose capability could not be read:
/// a keyed caller then refuses or waits for the next event rather than go keyless (spec 4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PresentedKey {
    Absent,
    Present(ContinuityKey),
    Unknown,
}

impl PresentedKey {
    /// The key a resolve sends: `Unknown` refuses with the retryable `ServiceBusy` before anything
    /// is journaled or sent, never a keyless resolve.
    pub(crate) fn for_resolve(self) -> Result<Option<ContinuityKey>, ApiError> {
        match self {
            Self::Absent => Ok(None),
            Self::Present(key) => Ok(Some(key)),
            Self::Unknown => Err(ApiError::new(ErrorCode::ServiceBusy, KEY_CAPABILITY_UNREAD)),
        }
    }
}

/// What this daemon says about `seat.continuity_key_v1`: one reading serves every key a caller
/// presents to it, so a hook asks once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyCapability {
    Advertised,
    Lacking,
    Unread,
}

/// The daemon's key capability. The client caches a failed read as no capabilities, so a daemon
/// not known to advertise is asked afresh: only a definitive answer is a no. A fresh list naming the
/// capability is `Unread` too: this process's cached reading stands for every other key it sends,
/// and the next call reads again.
pub(crate) fn key_capability<C: LocalClient + ?Sized>(
    client: &C,
    budget: &CallBudget,
) -> KeyCapability {
    if client.supports_capability(SEAT_CONTINUITY_KEY_V1, budget) {
        return KeyCapability::Advertised;
    }
    match client.call_definitive(Command::Capabilities, budget) {
        Ok(Ok(CommandResult::Capabilities(list)))
            if !list
                .capabilities
                .iter()
                .any(|c| c == SEAT_CONTINUITY_KEY_V1) =>
        {
            KeyCapability::Lacking
        }
        Ok(Err(error))
            if matches!(
                error.code,
                ErrorCode::UnknownWireVersion | ErrorCode::Unsupported
            ) =>
        {
            KeyCapability::Lacking
        }
        _ => KeyCapability::Unread,
    }
}

impl KeyCapability {
    /// The presentation of `env`'s key ([`caller_continuity_key`]) under this reading.
    pub(crate) fn present(self, env: &CallerEnv, top_level: bool) -> PresentedKey {
        match self {
            Self::Advertised => caller_continuity_key(env, top_level, true)
                .map_or(PresentedKey::Absent, PresentedKey::Present),
            Self::Unread if env.may_present_key(top_level) => PresentedKey::Unknown,
            Self::Lacking | Self::Unread => PresentedKey::Absent,
        }
    }
}

/// The presentation of a caller's key ([`caller_continuity_key`]). A caller with no valid key and a
/// subagent ask the daemon nothing, so their requests and round trips are unchanged.
pub(crate) fn continuity_key_presentation<C: LocalClient + ?Sized>(
    env: &CallerEnv,
    top_level: bool,
    client: &C,
    budget: &CallBudget,
) -> PresentedKey {
    if !env.may_present_key(top_level) {
        return PresentedKey::Absent;
    }
    key_capability(client, budget).present(env, top_level)
}

/// The key a plain CLI resolve of `target` presents: the caller's own pane on its own server only,
/// never a pane it resolves for someone else. Unknown or subagent callers present nothing. A plain
/// call offers no evidence to reattach on, so a key bound elsewhere refuses and mints no seat
/// (spec 4.2). An unread capability refuses ([`PresentedKey::for_resolve`]).
pub(crate) fn own_pane_continuity_key<C: LocalClient + ?Sized>(
    env: &CallerEnv,
    target: &HostTargetId,
    top_level: bool,
    on_own_server: bool,
    client: &C,
    budget: &CallBudget,
) -> Result<Option<ContinuityKey>, ApiError> {
    continuity_key_presentation(
        env,
        top_level && env.is_own_pane(target, on_own_server),
        client,
        budget,
    )
    .for_resolve()
}

/// The key of a resolve that runs outside the caller guard, so `caller::active` no longer answers:
/// its top-level test derives the origin from `parsed`.
pub(crate) fn resolve_arm_key<C: LocalClient + ?Sized>(
    parsed: &ParsedCli,
    env: &CallerEnv,
    target: &HostTargetId,
    on_own_server: bool,
    client: &C,
    budget: &CallBudget,
) -> Result<Option<ContinuityKey>, ApiError> {
    if !env.may_present_key(true) {
        return Ok(None);
    }
    let origin = caller::derive(parsed);
    own_pane_continuity_key(
        env,
        target,
        caller::is_top_level(origin.as_ref()),
        on_own_server,
        client,
        budget,
    )
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
        results::{CapabilityList, CommandResult},
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
        fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
            assert!(
                matches!(command, Command::Capabilities),
                "the gate sends no other command"
            );
            Ok(CommandResult::Capabilities(CapabilityList {
                capabilities: self.0.iter().map(|name| (*name).to_owned()).collect(),
            }))
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
            continuity_key: None,
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

    fn keyed_env(key: &str) -> CallerEnv {
        let key = key.to_owned();
        CallerEnv::from_lookup(move |name| match name {
            super::super::caller_location::CONTINUITY_KEY_ENV => Some(key.clone().into()),
            crate::host::native::ENV_PANE_ID => Some("w1:p1".into()),
            _ => None,
        })
    }

    fn presented(env: &CallerEnv, top_level: bool, client: &impl LocalClient) -> PresentedKey {
        continuity_key_presentation(env, top_level, client, &budget())
    }

    #[test]
    fn a_valid_key_is_presented_by_a_top_level_caller_of_an_advertising_daemon() {
        let key = presented(
            &keyed_env("clone-1"),
            true,
            &Fake(&[SEAT_CONTINUITY_KEY_V1]),
        );
        assert_eq!(key, PresentedKey::Present(ContinuityKey::new("clone-1")));
    }

    // An invalid value is absent, with no diagnostic and no round trip: `Panics` fails any question.
    #[test]
    fn an_invalid_value_is_absent_and_asks_nothing() {
        for value in ["", "has space", &"k".repeat(129)] {
            let env = keyed_env(value);
            assert_eq!(
                presented(&env, true, &Panics),
                PresentedKey::Absent,
                "{value:?}"
            );
        }
        // No variable at all: a keyless caller's round trips are unchanged.
        let keyless = CallerEnv::from_lookup(|_| None);
        assert_eq!(presented(&keyless, true, &Panics), PresentedKey::Absent);
    }

    #[test]
    fn a_subagent_presents_nothing_and_asks_nothing() {
        assert_eq!(
            presented(&keyed_env("clone-1"), false, &Panics),
            PresentedKey::Absent
        );
    }

    #[test]
    fn a_daemon_without_the_capability_gets_no_key() {
        assert_eq!(
            presented(&keyed_env("clone-1"), true, &Fake(&[])),
            PresentedKey::Absent
        );
    }

    #[test]
    fn the_helper_itself_gates_on_top_level_and_advertised() {
        let env = keyed_env("clone-1");
        assert_eq!(
            caller_continuity_key(&env, true, true),
            Some(ContinuityKey::new("clone-1"))
        );
        assert_eq!(caller_continuity_key(&env, false, true), None);
        assert_eq!(caller_continuity_key(&env, true, false), None);
    }

    // `launch` and recipient allocation resolve other panes: only the caller's own pane is keyed.
    #[test]
    fn a_plain_resolve_presents_the_key_only_for_the_callers_own_pane() {
        let env = keyed_env("clone-1");
        let client = Fake(&[SEAT_CONTINUITY_KEY_V1]);
        assert_eq!(
            own_pane_continuity_key(
                &env,
                &HostTargetId::new("w1:p1"),
                true,
                true,
                &client,
                &budget()
            ),
            Ok(Some(ContinuityKey::new("clone-1")))
        );
        assert_eq!(
            own_pane_continuity_key(
                &env,
                &HostTargetId::new("w1:p2"),
                true,
                true,
                &client,
                &budget()
            ),
            Ok(None)
        );
        assert_eq!(
            own_pane_continuity_key(
                &env,
                &HostTargetId::new("w1:p1"),
                false,
                true,
                &client,
                &budget()
            ),
            Ok(None)
        );
        // The same pane id on another server is not the caller's: `Panics` fails any question.
        assert_eq!(
            own_pane_continuity_key(
                &env,
                &HostTargetId::new("w1:p1"),
                true,
                false,
                &Panics,
                &budget()
            ),
            Ok(None)
        );
    }

    /// Advertises nothing (its read failed or the answer had no capability) and answers a fresh
    /// capability call with the scripted outcome.
    struct Fresh(fn() -> Result<Result<CommandResult, ApiError>, ApiError>);
    impl LocalClient for Fresh {
        fn supports_capability(&self, _: &str, _: &CallBudget) -> bool {
            false
        }
        fn call(&self, _: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
            unreachable!("a fresh read goes through call_definitive")
        }
        fn call_with_output(
            &self,
            _: Command,
            _: &crate::protocol::output::OutputSpec,
            _: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            unreachable!("a fresh read goes through call_definitive")
        }
        fn call_definitive(
            &self,
            command: Command,
            _: &CallBudget,
        ) -> Result<Result<CommandResult, ApiError>, ApiError> {
            assert!(matches!(command, Command::Capabilities));
            (self.0)()
        }
    }

    fn presentation(client: &impl LocalClient) -> PresentedKey {
        continuity_key_presentation(&keyed_env("clone-1"), true, client, &budget())
    }

    #[test]
    fn a_failed_capability_read_is_unknown() {
        let transport = Fresh(|| Err(ApiError::new(ErrorCode::DeadlineExceeded, "lost")));
        assert_eq!(presentation(&transport), PresentedKey::Unknown);
        let busy = Fresh(|| Ok(Err(ApiError::new(ErrorCode::ServiceBusy, "booting"))));
        assert_eq!(presentation(&busy), PresentedKey::Unknown);
        // Not a list: the daemon answered nothing about the capability.
        let other = Fresh(|| Ok(Ok(CommandResult::SeatResolved(SeatId::new("s")))));
        assert_eq!(presentation(&other), PresentedKey::Unknown);
        // The cached reading was no: this process waits for the next event's read.
        let naming = Fresh(|| {
            Ok(Ok(CommandResult::Capabilities(CapabilityList {
                capabilities: vec![SEAT_CONTINUITY_KEY_V1.to_owned()],
            })))
        });
        assert_eq!(presentation(&naming), PresentedKey::Unknown);
    }

    #[test]
    fn a_definitive_no_is_absent() {
        assert_eq!(presentation(&Fake(&[])), PresentedKey::Absent);
        let older = Fresh(|| Ok(Err(ApiError::new(ErrorCode::UnknownWireVersion, "old"))));
        assert_eq!(presentation(&older), PresentedKey::Absent);
        let refused = Fresh(|| Ok(Err(ApiError::new(ErrorCode::Unsupported, "old"))));
        assert_eq!(presentation(&refused), PresentedKey::Absent);
        assert_eq!(
            presentation(&Fake(&[SEAT_CONTINUITY_KEY_V1])),
            PresentedKey::Present(ContinuityKey::new("clone-1"))
        );
    }

    #[test]
    fn a_keyless_caller_or_subagent_is_absent_and_asks_nothing() {
        let keyless = CallerEnv::from_lookup(|_| None);
        assert_eq!(
            continuity_key_presentation(&keyless, true, &Panics, &budget()),
            PresentedKey::Absent
        );
        assert_eq!(
            continuity_key_presentation(&keyed_env("clone-1"), false, &Panics, &budget()),
            PresentedKey::Absent
        );
    }

    // The resolve arm runs after `ActiveGuard` dropped, so `active_is_top_level` answers true here.
    #[test]
    fn a_subagents_seat_resolve_presents_no_key() {
        use crate::cli::caller::{CallerOrigin, fix_origin};
        use crate::harness::caller_env::EnvClaim;
        use crate::protocol::{
            authority::{CallerRole, Harness, LivenessFingerprint},
            ids::{AgentId, NativeSessionId},
        };
        let claim = |role, agent_id| {
            CallerOrigin::Agent(EnvClaim {
                harness: Harness::Claude,
                native_session: NativeSessionId::new("sess-1"),
                agent_id,
                role,
                fingerprint: LivenessFingerprint::TEST,
                provenance: "test",
            })
        };
        let parsed = crate::cli::commands::parse_argv([
            "herdr-threads",
            "seat",
            "resolve",
            "--pane",
            "w1:p1",
        ])
        .unwrap();
        let key = |client: &dyn LocalClient| {
            resolve_arm_key(
                &parsed,
                &keyed_env("clone-1"),
                &HostTargetId::new("w1:p1"),
                true,
                client,
                &budget(),
            )
        };
        let client = Fake(&[SEAT_CONTINUITY_KEY_V1]);
        {
            let _origin = fix_origin(claim(CallerRole::Subagent, Some(AgentId::new("agent-1"))));
            assert!(crate::cli::caller::active_is_top_level(), "no guard here");
            assert_eq!(key(&Panics), Ok(None));
        }
        let _origin = fix_origin(claim(CallerRole::TopLevel, None));
        assert_eq!(key(&client), Ok(Some(ContinuityKey::new("clone-1"))));
    }

    // Kills: an unread capability read as no key, which sends a keyless `ResolveSeat` and mints a
    // hookless keyed clone a second seat.
    #[test]
    fn own_pane_and_resolve_arm_refuse_on_an_unread_capability() {
        let transport = Fresh(|| Err(ApiError::new(ErrorCode::DeadlineExceeded, "lost")));
        let own = HostTargetId::new("w1:p1");
        let refused = |result: Result<Option<ContinuityKey>, ApiError>| {
            let error = result.expect_err("refused, never Ok(None)");
            assert_eq!(error.code, ErrorCode::ServiceBusy);
            assert_eq!(error.detail, KEY_CAPABILITY_UNREAD);
        };
        refused(own_pane_continuity_key(
            &keyed_env("clone-1"),
            &own,
            true,
            true,
            &transport,
            &budget(),
        ));
        let parsed = crate::cli::commands::parse_argv([
            "herdr-threads",
            "seat",
            "resolve",
            "--pane",
            "w1:p1",
        ])
        .unwrap();
        refused(resolve_arm_key(
            &parsed,
            &keyed_env("clone-1"),
            &own,
            true,
            &transport,
            &budget(),
        ));
        // A keyless caller asks nothing and sends no key.
        let keyless = CallerEnv::from_lookup(|_| None);
        assert_eq!(
            own_pane_continuity_key(&keyless, &own, true, true, &Panics, &budget()),
            Ok(None)
        );
        assert_eq!(
            resolve_arm_key(&parsed, &keyless, &own, true, &Panics, &budget()),
            Ok(None)
        );
        // Another pane is never keyed, so an unread capability does not refuse it.
        assert_eq!(
            own_pane_continuity_key(
                &keyed_env("clone-1"),
                &HostTargetId::new("w1:p2"),
                true,
                true,
                &Panics,
                &budget()
            ),
            Ok(None)
        );
        assert_eq!(KEY_CAPABILITY_UNREAD.len(), 80);
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
