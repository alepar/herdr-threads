//! Precedence matrix and per-site answers of `cli::caller_location`.

use std::ffi::OsString;

use herdr_threads::cli::caller_location::{
    CallerEnv, CallerLocator, EnvMode, Located, LocatorError,
};

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

fn pane(id: &str) -> CallerLocator {
    CallerLocator::pane(id)
}

fn named(name: &str) -> CallerLocator {
    CallerLocator::Named(herdr_threads::protocol::ids::SeatName::parse(name).unwrap())
}

fn terminal(socket: &str, id: &str) -> CallerLocator {
    CallerLocator::Terminal {
        socket: socket.into(),
        id: id.into(),
    }
}

#[test]
fn precedence_matrix() {
    use EnvMode::{Cli, Hook};
    type Cell = (
        &'static [(&'static str, &'static str)],
        EnvMode,
        Result<Option<Located>, LocatorError>,
    );
    let cells: Vec<Cell> = vec![
        // nothing
        (&[], Cli, Ok(None)),
        (&[], Hook, Ok(None)),
        // pane only: the CLI takes it, a hook needs HERDR_ENV=1
        (
            &[("HERDR_PANE_ID", "w1:p1")],
            Cli,
            Ok(Some(Located::One(pane("w1:p1")))),
        ),
        (&[("HERDR_PANE_ID", "w1:p1")], Hook, Ok(None)),
        (
            &[("HERDR_PANE_ID", "w1:p1"), ("HERDR_ENV", "1")],
            Hook,
            Ok(Some(Located::One(pane("w1:p1")))),
        ),
        (
            &[("HERDR_PANE_ID", "w1:p1"), ("HERDR_ENV", "0")],
            Hook,
            Ok(None),
        ),
        // empty values are filtered
        (&[("HERDR_PANE_ID", "")], Cli, Ok(None)),
        (&[("HERDR_PANE_ID", ""), ("HERDR_ENV", "1")], Hook, Ok(None)),
        (&[("HERDR_ENV", "1")], Hook, Ok(None)),
        // terminal only
        (
            &[("HC_TERMINAL_ID", "t1"), ("HC_TERMINAL_SOCKET", "/s")],
            Cli,
            Ok(Some(Located::One(terminal("/s", "t1")))),
        ),
        // missing socket: refused when it is the only evidence
        (
            &[("HC_TERMINAL_ID", "t1")],
            Cli,
            Err(LocatorError::TerminalWithoutSocket),
        ),
        (
            &[("HC_TERMINAL_ID", "t1"), ("HC_TERMINAL_SOCKET", "")],
            Cli,
            Err(LocatorError::TerminalWithoutSocket),
        ),
        // a socket without an id is nothing
        (&[("HC_TERMINAL_SOCKET", "/s")], Cli, Ok(None)),
        // missing socket beside a pane: the pane stands
        (
            &[("HERDR_PANE_ID", "w1:p1"), ("HC_TERMINAL_ID", "t1")],
            Cli,
            Ok(Some(Located::One(pane("w1:p1")))),
        ),
        // both families
        (
            &[
                ("HERDR_PANE_ID", "w1:p1"),
                ("HC_TERMINAL_ID", "t1"),
                ("HC_TERMINAL_SOCKET", "/s"),
            ],
            Cli,
            Ok(Some(Located::Ambiguous(vec![
                pane("w1:p1"),
                terminal("/s", "t1"),
            ]))),
        ),
        // a hook without HERDR_ENV does not count the pane: only the terminal
        (
            &[
                ("HERDR_PANE_ID", "w1:p1"),
                ("HC_TERMINAL_ID", "t1"),
                ("HC_TERMINAL_SOCKET", "/s"),
            ],
            Hook,
            Ok(Some(Located::One(terminal("/s", "t1")))),
        ),
        (
            &[
                ("HERDR_PANE_ID", "w1:p1"),
                ("HERDR_ENV", "1"),
                ("HC_TERMINAL_ID", "t1"),
                ("HC_TERMINAL_SOCKET", "/s"),
            ],
            Hook,
            Ok(Some(Located::Ambiguous(vec![
                pane("w1:p1"),
                terminal("/s", "t1"),
            ]))),
        ),
        // precedence rule 1: a valid name replaces only the target, in either mode, with no
        // Herdr environment, and over every other family
        (
            &[("HERDR_THREADS_SEAT_NAME", "worker-1")],
            Hook,
            Ok(Some(Located::One(named("worker-1")))),
        ),
        (
            &[("HERDR_THREADS_SEAT_NAME", "worker-1")],
            Cli,
            Ok(Some(Located::One(named("worker-1")))),
        ),
        (
            &[
                ("HERDR_THREADS_SEAT_NAME", "w.1_x"),
                ("HERDR_PANE_ID", "w1:p1"),
                ("HERDR_ENV", "1"),
                ("HC_TERMINAL_ID", "t1"),
                ("HC_TERMINAL_SOCKET", "/s"),
            ],
            Hook,
            Ok(Some(Located::One(named("w.1_x")))),
        ),
        // an empty name is filtered like any other value
        (&[("HERDR_THREADS_SEAT_NAME", "")], Cli, Ok(None)),
        // an invalid name is a hard error, even beside a pane
        (
            &[("HERDR_THREADS_SEAT_NAME", "-bad")],
            Cli,
            Err(LocatorError::InvalidName),
        ),
        (
            &[
                ("HERDR_THREADS_SEAT_NAME", "has space"),
                ("HERDR_PANE_ID", "w1:p1"),
                ("HERDR_ENV", "1"),
            ],
            Hook,
            Err(LocatorError::InvalidName),
        ),
        // the read-only marker alone is not a name
        (&[("HERDR_THREADS_SEAT_READONLY", "1")], Cli, Ok(None)),
    ];
    for (vars, mode, expected) in cells {
        assert_eq!(env(vars).locate(mode), expected, "{vars:?} {mode:?}");
    }
}

#[test]
fn pane_candidate_is_used_when_ambiguous_and_terminal_alone_is_no_pane() {
    let both = env(&[
        ("HERDR_PANE_ID", "w1:p1"),
        ("HC_TERMINAL_ID", "t1"),
        ("HC_TERMINAL_SOCKET", "/s"),
    ]);
    assert_eq!(both.pane(EnvMode::Cli).as_deref(), Some("w1:p1"));
    let term = env(&[("HC_TERMINAL_ID", "t1"), ("HC_TERMINAL_SOCKET", "/s")]);
    assert_eq!(term.pane(EnvMode::Cli), None);
    let refused = env(&[("HC_TERMINAL_ID", "t1")]);
    assert_eq!(refused.pane(EnvMode::Cli), None);
}

#[test]
fn herdr_env_is_exactly_one() {
    assert!(env(&[("HERDR_ENV", "1")]).herdr_env());
    assert!(!env(&[("HERDR_ENV", "true")]).herdr_env());
    assert!(!env(&[("HERDR_ENV", "")]).herdr_env());
    assert!(!env(&[]).herdr_env());
}

#[test]
fn pane_site_answers() {
    let locator = pane("w4:p1");
    let site = locator.site();
    assert_eq!(site.moved_target(), Some("w4:p1"));
    assert_eq!(site.relative_pane(), Some("w4:p1"));
    // Byte-identical to the pre-F recovery hint word.
    assert_eq!(
        site.env_assignments(),
        vec!["HERDR_PANE_ID=w4:p1".to_owned()]
    );
}

#[test]
fn terminal_site_is_not_a_pane_and_scopes_its_lookup() {
    let locator = terminal("/s", "t1");
    let site = locator.site();
    assert_eq!(site.relative_pane(), None);
    assert_eq!(site.required_capability(), Some("seat.terminal_sites_v1"));
    let (host, target) = site.sent_scope().unwrap();
    assert_eq!((host.as_str(), target.as_str()), ("/s", "t1"));
}

#[test]
fn named_site_answers() {
    let locator = named("worker-1");
    let site = locator.site();
    assert_eq!(site.moved_target(), Some("worker-1"));
    assert_eq!(site.relative_pane(), None);
    assert_eq!(site.locate_candidate(), None);
    assert_eq!(site.terminal_address(), None);
    assert_eq!(
        site.claim_host_locator(),
        Some("/.herdr-threads/named-host")
    );
}

#[test]
fn named_env_assignments_hint() {
    let locator = named("worker-1");
    assert_eq!(
        locator.site().env_assignments(),
        vec!["HERDR_THREADS_SEAT_NAME=worker-1".to_owned()]
    );
}

#[test]
fn a_named_caller_has_no_pane() {
    let e = env(&[
        ("HERDR_THREADS_SEAT_NAME", "worker-1"),
        ("HERDR_PANE_ID", "w1:p1"),
    ]);
    assert_eq!(e.pane(EnvMode::Cli), None);
    assert_eq!(e.named().unwrap().as_str(), "worker-1");
    assert!(
        env(&[("HERDR_THREADS_SEAT_NAME", "-bad")])
            .named()
            .is_none()
    );
}

#[test]
fn readonly_marker_is_exactly_one() {
    assert!(env(&[("HERDR_THREADS_SEAT_READONLY", "1")]).readonly());
    assert!(!env(&[("HERDR_THREADS_SEAT_READONLY", "0")]).readonly());
    assert!(!env(&[("HERDR_THREADS_SEAT_READONLY", "true")]).readonly());
    assert!(!env(&[]).readonly());
    // It never changes where the caller is: a pane site stays a pane site.
    let both = env(&[
        ("HERDR_THREADS_SEAT_READONLY", "1"),
        ("HERDR_PANE_ID", "w1:p1"),
    ]);
    assert_eq!(
        both.locate(EnvMode::Cli),
        Ok(Some(Located::One(pane("w1:p1"))))
    );
}

#[test]
fn terminal_site_answers() {
    let locator = terminal("/run/herdr-core/work.sock", "t3");
    let site = locator.site();
    assert_eq!(site.moved_target(), Some("t3"));
    assert_eq!(site.relative_pane(), None);
    assert_eq!(site.terminal_address().as_deref(), Some("work:t3"));
    // The core's own socket is the claim host; the installed Herdr endpoint never is.
    assert_eq!(site.claim_host_locator(), Some("/run/herdr-core/work.sock"));
    assert_eq!(
        site.env_assignments(),
        vec![
            "HC_TERMINAL_ID=t3".to_owned(),
            "HC_TERMINAL_SOCKET=/run/herdr-core/work.sock".to_owned()
        ]
    );
    // A socket with no file stem leaves the bare terminal id.
    assert_eq!(
        terminal("/", "t3").site().terminal_address().as_deref(),
        Some("t3")
    );
    assert_eq!(pane("w1:p1").site().claim_host_locator(), None);
}

/// The lone non-Pane callers instance discovery treats alike; a pane family anywhere makes the
/// caller a Pane site (Herdr's own discovery applies).
#[test]
fn non_pane_site_is_a_lone_name_or_terminal() {
    let term = [("HC_TERMINAL_ID", "t1"), ("HC_TERMINAL_SOCKET", "/s")];
    assert!(env(&term).non_pane_site());
    assert!(env(&[("HERDR_THREADS_SEAT_NAME", "worker-1")]).non_pane_site());
    assert!(!env(&[("HERDR_PANE_ID", "w1:p1")]).non_pane_site());
    assert!(!env(&[]).non_pane_site());
    assert!(
        !env(&[
            ("HC_TERMINAL_ID", "t1"),
            ("HC_TERMINAL_SOCKET", "/s"),
            ("HERDR_PANE_ID", "w1:p1")
        ])
        .non_pane_site()
    );
    // The refusal is not a site.
    assert!(!env(&[("HC_TERMINAL_ID", "t1")]).non_pane_site());
}

/// Kills: a terminal id with no socket silently treated as no caller, instead of the M17 refusal;
/// M17 beside a pane (the pane stands) or with nothing at all.
#[test]
fn terminal_without_socket_m17() {
    use herdr_threads::cli::caller_location::M17;
    let refusal = |pairs: &[(&str, &str)]| {
        env(pairs)
            .locate(EnvMode::Cli)
            .err()
            .and_then(|error| error.refusal())
    };
    assert_eq!(refusal(&[("HC_TERMINAL_ID", "t1")]), Some(M17));
    assert_eq!(
        refusal(&[("HC_TERMINAL_ID", "t1"), ("HC_TERMINAL_SOCKET", "")]),
        Some(M17)
    );
    assert_eq!(
        M17,
        "herdr-threads: HC_TERMINAL_ID is set without HC_TERMINAL_SOCKET; cannot locate the terminal"
    );
    assert_eq!(M17.len(), 91);
    assert_eq!(
        refusal(&[("HC_TERMINAL_ID", "t1"), ("HERDR_PANE_ID", "w1:p1")]),
        None
    );
    assert_eq!(refusal(&[]), None);
}

// Kills: a hint without a seat name (a plain tmux shell sends none), a hint built from a garbled
// or empty TMUX/TMUX_PANE, a hint that is the Herdr pane locator, and a hint that changes the
// site or the Herdr locator hint.
#[test]
fn tmux_hint_needs_a_named_caller_in_a_tmux_pane() {
    let tmux = [
        ("TMUX", "/tmp/tmux-1000/default,4242,0"),
        ("TMUX_PANE", "%3"),
    ];
    let named = [("HERDR_THREADS_SEAT_NAME", "reviewer")];
    let hint = |pairs: &[&[(&str, &str)]]| env(&pairs.concat()).tmux_hint();
    assert_eq!(
        hint(&[&named, &tmux]).as_deref(),
        Some("tmux:/tmp/tmux-1000/default:4242:%3")
    );
    assert_eq!(hint(&[&tmux]), None, "no seat name");
    assert_eq!(hint(&[&named]), None, "not in tmux");
    assert_eq!(
        hint(&[&named, &[("TMUX", "/tmp/s,4242,0")]]),
        None,
        "no pane"
    );
    assert_eq!(
        hint(&[&named, &[("TMUX", ""), ("TMUX_PANE", "%3")]]),
        None,
        "empty TMUX"
    );
    assert_eq!(
        hint(&[&named, &[("TMUX", "/tmp/s,x,0"), ("TMUX_PANE", "%3")]]),
        None
    );
    // Beside a Herdr pane the tmux hint is separate from the locator hint and the site.
    let both = env(&[&named[..], &tmux[..], &[("HERDR_PANE_ID", "w1:p1")]].concat());
    assert_eq!(both.locator_hint().as_deref(), Some("w1:p1"));
    assert_eq!(
        both.tmux_hint().as_deref(),
        Some("tmux:/tmp/tmux-1000/default:4242:%3")
    );
    assert!(matches!(
        both.locate(EnvMode::Cli).unwrap(),
        Some(Located::One(CallerLocator::Named(_)))
    ));
}

// Kills: a locator hint that drops the core socket (two cores sharing `tN` look alike) or sends
// only one of the candidates a named caller has.
#[test]
fn locator_hint_carries_socket_and_every_candidate() {
    let named = [("HERDR_THREADS_SEAT_NAME", "reviewer")];
    let terminal = [
        ("HC_TERMINAL_ID", "t4"),
        ("HC_TERMINAL_SOCKET", "/run/hc/core-a.sock"),
    ];
    let hint = |pairs: &[&[(&str, &str)]]| env(&pairs.concat()).locator_hint();
    assert_eq!(
        hint(&[&named, &terminal]).as_deref(),
        Some("/run/hc/core-a.sock:t4")
    );
    assert_eq!(
        hint(&[&named, &terminal, &[("HERDR_PANE_ID", "w1:p2")]]).as_deref(),
        Some("w1:p2\n/run/hc/core-a.sock:t4")
    );
    assert_eq!(
        hint(&[&named, &[("HC_TERMINAL_ID", "t4")]]),
        None,
        "an id with no socket names no core"
    );
    assert_eq!(hint(&[&terminal]), None, "no name, no hint");
}

// Kills: a caller-location variable (`HC_TERMINAL_ID`, `TMUX`, ...) missing from the test scrub, so
// the suite's result depends on whether the runner sits in a herdr-core terminal or a tmux pane.
#[test]
fn every_caller_location_variable_is_scrubbed() {
    use herdr_threads::host::{hc, named, native};
    use herdr_threads::test_support::isolation::SCRUBBED_PREFIXES;
    for name in native::ENV_VARS
        .iter()
        .chain(named::ENV_VARS)
        .chain(hc::ENV_VARS)
    {
        assert!(
            SCRUBBED_PREFIXES.iter().any(|p| name.starts_with(p)),
            "{name} is a caller-location variable but no SCRUBBED_PREFIXES entry covers it"
        );
    }
}

/// Kills: a verified terminal that still reads as two families (the pane survives), one that
/// loses the Herdr environment instance discovery needs, and a Hook reading that differs.
#[test]
fn verified_terminal_reads_as_a_lone_terminal() {
    let both = env(&[
        ("HERDR_PANE_ID", "w1:p1"),
        ("HERDR_ENV", "1"),
        ("HC_TERMINAL_ID", "t3"),
        ("HC_TERMINAL_SOCKET", "/s"),
    ]);
    assert!(matches!(
        both.locate(EnvMode::Cli),
        Ok(Some(Located::Ambiguous(_)))
    ));
    let verified = both.verified_terminal();
    for mode in [EnvMode::Cli, EnvMode::Hook] {
        assert_eq!(
            verified.locate(mode),
            Ok(Some(Located::One(terminal("/s", "t3"))))
        );
    }
    assert!(verified.herdr_env());
    assert_eq!(verified.pane(EnvMode::Cli), None);
}

/// Kills: a scope that stays set after it drops (every later read in the thread loses its pane),
/// and a nested scope whose drop clears the outer one.
#[test]
fn verified_terminal_scope_restores_on_drop() {
    use herdr_threads::cli::caller_location::{enter_verified_terminal, verified_terminal_active};
    assert!(!verified_terminal_active());
    let outer = enter_verified_terminal();
    assert!(verified_terminal_active());
    drop(enter_verified_terminal());
    assert!(
        verified_terminal_active(),
        "the inner drop keeps the outer scope"
    );
    drop(outer);
    assert!(!verified_terminal_active());
}
