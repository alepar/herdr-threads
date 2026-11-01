//! A herdr-core terminal caller site (ht-kqrt.46, H1e): its instance discovery is that of a
//! named site (the Herdr command group's store beside a Herdr), its hook claim host is the
//! core's socket rather than the Herdr endpoint baked into the installed argv, and `doctor`
//! flags the second store a split would leave. A stub `herdr` on PATH stands in for Herdr.

use super::instance_named::{DEFAULT, scratch};
use super::*;
use crate::cli::caller_location::CallerEnv;
use std::{ffi::OsString, fs};

fn env(pairs: &'static [(&'static str, &'static str)]) -> CallerEnv {
    CallerEnv::from_lookup(move |name| {
        pairs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| OsString::from(v))
    })
}

const TERMINAL: &[(&str, &str)] = &[
    ("HC_TERMINAL_ID", "t3"),
    ("HC_TERMINAL_SOCKET", "/core/work.sock"),
];

/// Kills: a terminal caller landing in the standalone default dir beside a Herdr (a split
/// store) instead of the command group's store, and an eager `status server` query for the host.
/// The store is found on the fast filesystem default (no stub `herdr`, so no script race).
#[test]
fn terminal_site_uses_herdr_command_group_store() {
    let root = scratch("term-store");
    let xdg = root.join("st");
    let store = xdg.join("herdr/plugins/herdr-threads");
    fs::create_dir_all(store.join("node")).unwrap();
    fs::write(
        store.join("node").join(crate::daemon::paths::DATABASE_FILE),
        b"",
    )
    .unwrap();
    let inputs = InstanceInputs {
        home: Some(root.join("home")),
        xdg_state_home: Some(xdg),
        herdr_env: true,
        non_pane_site: env(TERMINAL).non_pane_site(),
        ..InstanceInputs::default()
    };
    assert!(inputs.non_pane_site, "a lone terminal is a non-Pane site");
    assert_eq!(default_state_dir_to_create(&inputs), Ok(None));
    let (found, how) = resolve_state_dir(&inputs).unwrap();
    assert_eq!(found, store);
    assert_ne!(how, STANDALONE_HOW);
    assert!(!root.join("home").join(DEFAULT).exists());
    // With no Herdr to ask, the host stays unresolved rather than invented.
    assert!(resolve_host_endpoint(&inputs).is_err());
    fs::remove_dir_all(&root).unwrap();
}

/// Kills: a verified two-family caller whose instance discovery keeps the Pane-site rules (or
/// loses `HERDR_ENV`) and lands in a split standalone store instead of the command group's.
#[test]
fn verified_terminal_keeps_the_command_group_store() {
    let root = scratch("term-verified");
    let xdg = root.join("st");
    let store = xdg.join("herdr/plugins/herdr-threads");
    fs::create_dir_all(store.join("node")).unwrap();
    fs::write(
        store.join("node").join(crate::daemon::paths::DATABASE_FILE),
        b"",
    )
    .unwrap();
    let both = env(&[
        ("HERDR_PANE_ID", "w1:p1"),
        ("HERDR_ENV", "1"),
        ("HC_TERMINAL_ID", "t3"),
        ("HC_TERMINAL_SOCKET", "/core/work.sock"),
    ])
    .verified_terminal();
    assert!(both.herdr_env());
    let inputs = InstanceInputs {
        home: Some(root.join("home")),
        xdg_state_home: Some(xdg),
        herdr_env: both.herdr_env(),
        non_pane_site: both.non_pane_site(),
        ..InstanceInputs::default()
    };
    assert!(
        inputs.non_pane_site,
        "a verified terminal is a non-Pane site"
    );
    assert_eq!(default_state_dir_to_create(&inputs), Ok(None));
    assert_eq!(resolve_state_dir(&inputs).unwrap().0, store);
    fs::remove_dir_all(&root).unwrap();
}

/// Kills: a terminal caller on a machine with no Herdr that finds no state dir at all.
#[test]
fn terminal_site_without_herdr_gets_the_standalone_default() {
    let root = scratch("term-standalone");
    let inputs = InstanceInputs {
        home: Some(root.clone()),
        non_pane_site: env(TERMINAL).non_pane_site(),
        ..InstanceInputs::default()
    };
    let (context, source, ..) = resolve_context_with_selected_paths(&inputs).unwrap();
    assert_eq!(context.state_dir, root.join(DEFAULT));
    assert_eq!(source["state_dir"], "standalone default");
    fs::remove_dir_all(&root).unwrap();
}

/// Kills: a hook from a terminal that claims the Herdr TUI endpoint the installed argv bakes in
/// (the core is not that server), and a pane site whose own `HERDR_SOCKET_PATH` and baked
/// fallback order changed.
#[test]
fn baked_tui_host_endpoint_not_used_for_hc() {
    let root = scratch("term-claim");
    let sockets = root.join("sockets");
    fs::create_dir_all(&sockets).unwrap();
    fs::create_dir_all(root.join("state")).unwrap();
    let path = |name: &str| sockets.join(name).display().to_string();
    let (tui, core, stale, pane_socket) = (
        path("tui.sock"),
        path("work.sock"),
        path("stale.sock"),
        path("pane.sock"),
    );
    let args = crate::cli::hook::HookArgs {
        state_dir: Some(root.join("state")),
        host_endpoint: Some(PathBuf::from(&tui)),
        harness: crate::harness::registry::OccupantHarness::Agent(
            crate::harness::registry::builtins()
                .agent("claude")
                .unwrap(),
        )
        .into(),
        event: None,
    };
    let owned_env = |pairs: Vec<(&'static str, String)>| {
        CallerEnv::from_lookup(move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        })
    };
    let claim = |env: &CallerEnv, socket: Option<&str>| {
        crate::cli::hook::hook_runtime_context_in(&args, env, socket.map(OsString::from))
            .unwrap()
            .claim_host()
            .unwrap()
            .as_str()
            .to_owned()
    };
    // A terminal: its core socket, even beside a stale HERDR_SOCKET_PATH.
    let terminal = owned_env(vec![
        ("HC_TERMINAL_ID", "t3".into()),
        ("HC_TERMINAL_SOCKET", core.clone()),
    ]);
    assert_eq!(claim(&terminal, Some(&stale)), core);
    // A pane: its own socket first, else the baked endpoint.
    let pane = owned_env(vec![
        ("HERDR_ENV", "1".into()),
        ("HERDR_PANE_ID", "w1:p1".into()),
    ]);
    assert_eq!(claim(&pane, Some(&pane_socket)), pane_socket);
    assert_eq!(claim(&pane, None), tui);
    fs::remove_dir_all(&root).unwrap();
}

/// Kills: a doctor that does not see the standalone default store a terminal caller left beside
/// the command group's store (two stores for one user).
#[test]
fn doctor_flags_two_stores() {
    let root = scratch("term-two-stores");
    let xdg = root.join("st");
    let group = xdg.join("herdr/plugins/herdr-threads");
    let default = root.join("home").join(DEFAULT);
    for dir in [&group, &default] {
        fs::create_dir_all(dir.join("node")).unwrap();
        fs::write(
            dir.join("node").join(crate::daemon::paths::DATABASE_FILE),
            b"",
        )
        .unwrap();
    }
    // The terminal caller resolved the command group's store; the default dir is the split.
    let (xdg_candidate, home_candidate) = state_candidates(Some(&root.join("home")), Some(&xdg));
    assert_eq!(
        crate::cli::doctor::other_stores(&group, [xdg_candidate, home_candidate]),
        vec![default.display().to_string()]
    );
    fs::remove_dir_all(&root).unwrap();
}
