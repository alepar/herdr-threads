//! The Named caller site's command shape and its `seat.host_sites_v1` gate (M13).

use std::ffi::OsString;
use std::sync::Mutex;

use herdr_threads::cli::{
    caller_location::{CallerEnv, CallerLocator, EnvMode, Located},
    hook::HookEnv,
    site_capability::{M13, resolve_thread_query},
};
use herdr_threads::ports::LocalClient;
use herdr_threads::protocol::{
    capabilities::HOST_SITES_V1,
    commands::Command,
    ids::{HostTargetId, NAMED_HOST_LOCATOR, SeatId},
    output::OutputSpec,
    results::{ApiError, CommandResult, ErrorCode},
    time::{CallBudget, Cancellation, MonoInstant},
};

struct Daemon {
    caps: &'static [&'static str],
    sent: Mutex<Vec<Command>>,
}

impl Daemon {
    fn new(caps: &'static [&'static str]) -> Self {
        Self {
            caps,
            sent: Mutex::new(Vec::new()),
        }
    }
}

impl LocalClient for Daemon {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        self.caps.contains(&name)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.sent.lock().unwrap().push(command);
        Err(ApiError::new(ErrorCode::Unsupported, "not served"))
    }
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

fn env(pairs: &[(&str, &str)]) -> CallerEnv {
    let owned: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    CallerEnv::from_lookup(move |name| {
        owned
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| OsString::from(v))
    })
}

fn named_site(e: &CallerEnv, mode: EnvMode) -> CallerLocator {
    match e.locate(mode).unwrap() {
        Some(Located::One(site @ CallerLocator::Named(_))) => site,
        other => panic!("not a named site: {other:?}"),
    }
}

/// The CLI path (`EnvMode::Cli`) and the hook path (`EnvMode::Hook`) with the name set and no
/// `HERDR_ENV` both resolve the Named site; it sends the name as target at the reserved locator.
#[test]
fn named_resolve_command_shape() {
    let e = env(&[("HERDR_THREADS_SEAT_NAME", "worker-1")]);
    assert!(!e.herdr_env());
    for mode in [EnvMode::Cli, EnvMode::Hook] {
        let locator = named_site(&e, mode);
        let daemon = Daemon::new(&[HOST_SITES_V1]);
        let query = resolve_thread_query(
            Some(locator.site()),
            "ops",
            Some(SeatId::new("s")),
            Some(HostTargetId::new("w1:p1")),
            &daemon,
            &budget(),
        )
        .unwrap();
        assert_eq!(query.selector, "ops");
        assert_eq!(query.caller, Some(SeatId::new("s")));
        // The name replaces the pane target and rides the reserved locator.
        assert_eq!(query.caller_target, Some(HostTargetId::new("worker-1")));
        assert_eq!(query.caller_host.unwrap().as_str(), NAMED_HOST_LOCATOR);
        assert!(daemon.sent.lock().unwrap().is_empty());
    }
}

/// The hook environment of a named process is admitted without `HERDR_ENV` and acts for the name.
#[test]
fn named_hook_env_is_located_without_herdr_env() {
    let named = HookEnv {
        herdr_env: false,
        pane: None,
        ambiguous: None,
        named: Some(herdr_threads::protocol::ids::SeatName::parse("worker-1").unwrap()),
    };
    assert!(named.located());
    assert_eq!(named.target(), Some("worker-1"));
    assert_eq!(herdr_threads::cli::hook::foreign_session(&named), None);
    let outside = HookEnv {
        named: None,
        ..named.clone()
    };
    assert!(!outside.located());
    assert_eq!(outside.target(), None);
    // The name replaces only the target: a pane beside it is not the target.
    let both = HookEnv {
        herdr_env: true,
        pane: Some("w1:p1".into()),
        ..named
    };
    assert_eq!(both.target(), Some("worker-1"));
}

#[test]
fn named_site_against_old_daemon_prints_m13() {
    let locator = CallerLocator::Named(herdr_threads::protocol::ids::SeatName::parse("w").unwrap());
    let daemon = Daemon::new(&["other.cap"]);
    let error = resolve_thread_query(Some(locator.site()), "ops", None, None, &daemon, &budget())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
    assert_eq!(error.detail, M13);
    assert!(
        daemon.sent.lock().unwrap().is_empty(),
        "no Named site was sent"
    );
}

/// A Pane site and no site send no host and keep their pane target, against any daemon.
#[test]
fn pane_site_bytes_are_unchanged() {
    let locator = CallerLocator::pane("w1:p1");
    let daemon = Daemon::new(&[]);
    let query = resolve_thread_query(
        Some(locator.site()),
        "ops",
        None,
        Some(HostTargetId::new("w1:p1")),
        &daemon,
        &budget(),
    )
    .unwrap();
    assert_eq!(query.caller_target, Some(HostTargetId::new("w1:p1")));
    assert!(query.caller_host.is_none());
    let none = resolve_thread_query(None, "ops", None, None, &daemon, &budget()).unwrap();
    assert!(none.caller_target.is_none() && none.caller_host.is_none());
}

#[test]
fn m13_golden() {
    let golden = include_str!("golden/m13.txt");
    assert_eq!(format!("{M13}\n"), golden);
    assert_eq!(M13.len(), 63);
}

fn named_locator() -> CallerLocator {
    CallerLocator::Named(herdr_threads::protocol::ids::SeatName::parse("worker-1").unwrap())
}

fn run_named(argv: &[&str]) -> herdr_threads::cli::RunError {
    let mut out = Vec::new();
    let locator = named_locator();
    herdr_threads::cli::run_in_site(argv.iter().copied(), None, Some(&locator), &mut out)
        .unwrap_err()
}

fn detail(error: herdr_threads::cli::RunError) -> String {
    match error {
        herdr_threads::cli::RunError::Api(api) => api.detail,
        other => panic!("not an API error: {other:?}"),
    }
}

#[test]
fn m14_m15_m16_goldens() {
    use herdr_threads::cli::caller_location::{M15, M16, m14};
    assert_eq!(
        format!("{}\n", m14("--cooperative-seat")),
        include_str!("golden/m14.txt")
    );
    assert_eq!(m14("").len(), 63);
    assert_eq!(format!("{M15}\n"), include_str!("golden/m15.txt"));
    assert_eq!(M15.len(), 82);
    assert_eq!(format!("{M16}\n"), include_str!("golden/m16.txt"));
    assert_eq!(M16.len(), 92);
}

#[test]
fn invalid_name_maps_to_m16_and_terminal_without_socket_to_m17() {
    use herdr_threads::cli::caller_location::{LocatorError, M16, M17};
    for value in ["-bad", "has space", &"a".repeat(65)] {
        let e = env(&[("HERDR_THREADS_SEAT_NAME", value)]);
        let error = e.locate(EnvMode::Cli).unwrap_err();
        assert_eq!(error.refusal(), Some(M16), "{value}");
    }
    assert_eq!(LocatorError::TerminalWithoutSocket.refusal(), Some(M17));
}

#[test]
fn me_init_under_a_name_is_refused_with_m15() {
    let error = run_named(&["herdr-threads", "human", "me", "init"]);
    assert_eq!(detail(error), herdr_threads::cli::caller_location::M15);
}

#[test]
fn explicit_cooperative_flags_under_a_name_are_refused_with_m14() {
    let error = run_named(&[
        "herdr-threads",
        "--cooperative-seat",
        "s123",
        "--cooperative-target",
        "w1:p1",
        "--cooperative-harness",
        "claude",
        "--cooperative-role",
        "top-level",
        "inbox",
    ]);
    assert_eq!(
        detail(error),
        herdr_threads::cli::caller_location::m14("--cooperative-seat")
    );
}
