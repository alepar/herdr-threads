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
    assert!(site.person_origin_allowed());
    assert!(site.enrolls_on_first_use());
    assert!(site.attempts_continuity());
    assert!(site.host_scoped_lookup());
    assert_eq!(site.cache_key(), "pane:w4:p1");
    // Byte-identical to the pre-F recovery hint word.
    assert_eq!(
        site.env_assignments(),
        vec!["HERDR_PANE_ID=w4:p1".to_owned()]
    );
}

#[test]
fn recognition_only_terminal_site_is_not_a_pane() {
    let locator = terminal("/s", "t1");
    let site = locator.site();
    assert_eq!(site.relative_pane(), None);
    assert!(!site.person_origin_allowed());
    assert!(!site.enrolls_on_first_use());
}
