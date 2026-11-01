//! Instance discovery for Named sites (SA1): the standalone default state dir, the refusal to
//! split a store beside Herdr, and the lazy Herdr endpoint. A stub `herdr` on PATH stands in for
//! Herdr (no real Herdr server runs here; the isolated-session variant is not exercised).

use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

pub(super) fn scratch(tag: &str) -> PathBuf {
    let dir =
        crate::test_support::scratch::temp_path(format!("ht-inn-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A `herdr` that logs each call and answers `plugin list` with the installed plugin.
fn stub_herdr(dir: &Path) -> PathBuf {
    let bin = dir.join("herdr");
    fs::write(
        &bin,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$1 $2\" in\n\
             'plugin list') printf '%s' '{{\"result\":{{\"plugins\":[{{\"plugin_id\":\"herdr-threads\",\"enabled\":true}}]}}}}';;\n\
             *) exit 3;;\nesac\n",
            dir.join("herdr.calls").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn calls(dir: &Path) -> String {
    fs::read_to_string(dir.join("herdr.calls")).unwrap_or_default()
}

pub(super) const DEFAULT: &str = ".local/state/herdr/plugins/herdr-threads";

/// Kills: a named site that finds no state dir without Herdr, a dir created with the umask's
/// mode, and a host endpoint that is invented rather than absent.
#[test]
fn default_dir_created_without_herdr() {
    let root = scratch("create");
    let inputs = InstanceInputs {
        home: Some(root.clone()),
        non_pane_site: true,
        ..InstanceInputs::default()
    };
    let (context, source, ..) = resolve_context_with_selected_paths(&inputs).unwrap();
    let dir = root.join(DEFAULT);
    assert_eq!(context.state_dir, dir);
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(source["state_dir"], "standalone default");
    assert_eq!(source["host_endpoint"], "none");
    assert_eq!(context.host_endpoint, None);
    fs::remove_dir_all(&root).unwrap();
}

/// Kills: a default dir created beside a `herdr` on PATH (a split store), an eager
/// `status server` query for the host, and a missing `plugin list` query.
#[test]
fn default_dir_not_created_with_stub_herdr_on_path() {
    let root = scratch("stub");
    let inputs = InstanceInputs {
        home: Some(root.join("home")),
        herdr: Some(stub_herdr(&root)),
        non_pane_site: true,
        ..InstanceInputs::default()
    };
    assert_eq!(default_state_dir_to_create(&inputs), Ok(None));
    let _ = resolve_context_with_selected_paths(&inputs);
    assert!(!root.join("home").join(DEFAULT).exists());
    let calls = calls(&root);
    assert!(calls.contains("plugin list"), "{calls}");
    assert!(!calls.contains("status server"), "lazy host: {calls}");
    fs::remove_dir_all(&root).unwrap();
}

/// Kills: a named caller with Herdr env landing in the default dir instead of the command
/// group's store that `herdr plugin list` names (here the XDG plugin state root).
#[test]
fn named_caller_with_herdr_env_uses_command_group_store() {
    let root = scratch("beside");
    let xdg = root.join("st");
    let store = xdg.join("herdr/plugins/herdr-threads");
    fs::create_dir_all(store.join("node")).unwrap();
    let inputs = InstanceInputs {
        home: Some(root.join("home")),
        xdg_state_home: Some(xdg),
        herdr: Some(stub_herdr(&root)),
        herdr_env: true,
        non_default_server: true,
        non_pane_site: true,
        ..InstanceInputs::default()
    };
    assert_eq!(default_state_dir_to_create(&inputs), Ok(None));
    let (found, how) = resolve_state_dir(&inputs).unwrap();
    assert_eq!(found, store);
    assert!(how.starts_with("herdr plugin list"), "{how}");
    assert!(!root.join("home").join(DEFAULT).exists());
    fs::remove_dir_all(&root).unwrap();
}

/// Kills: Pane-site discovery that gained the standalone default: with no Herdr it must still
/// fail (and create nothing), exactly as before.
#[test]
fn pane_site_discovery_unchanged() {
    let root = scratch("pane");
    let inputs = InstanceInputs {
        home: Some(root.clone()),
        ..InstanceInputs::default()
    };
    assert!(resolve_state_dir(&inputs).is_err());
    assert!(resolve_context(&inputs).is_err());
    assert!(!root.join(".local").exists());
    // And `herdr` present but a Pane site: the eager host query still runs.
    let with_herdr = InstanceInputs {
        herdr: Some(stub_herdr(&root)),
        ..inputs
    };
    let _ = resolve_host_endpoint(&with_herdr);
    assert!(calls(&root).contains("status server"));
    fs::remove_dir_all(&root).unwrap();
}

/// Kills: a named CLI process whose claim host is the Herdr server (or nothing) instead of the
/// reserved locator, with or without a Herdr configured, and an explicit `--host-endpoint`
/// that displaces it (the hook path keys on the reserved locator, so the CLI must too).
#[test]
fn named_site_host_is_the_reserved_locator() {
    let root = scratch("reserved");
    let state = root.join("state");
    fs::create_dir(&state).unwrap();
    let reserved = PathBuf::from(crate::protocol::ids::NAMED_HOST_LOCATOR);
    let herdr_socket = root.join("herdr.sock");
    for (env_host, host_flag) in [
        (None, None),
        (Some(herdr_socket.clone()), None),
        (Some(herdr_socket.clone()), Some(root.join("flag.sock"))),
    ] {
        let inputs = InstanceInputs {
            home: Some(root.join("home")),
            state_flag: Some(state.clone()),
            env_host: env_host.clone(),
            host_flag,
            non_pane_site: true,
            site_host: Some(reserved.clone()),
            ..InstanceInputs::default()
        };
        let (context, source, ..) = resolve_context_with_selected_paths(&inputs).unwrap();
        assert_eq!(
            context.host_endpoint.as_ref(),
            Some(&reserved),
            "{env_host:?}"
        );
        assert_eq!(source["host_endpoint"], "the named site reserved locator");
    }
    fs::remove_dir_all(&root).unwrap();
}

/// Kills: a lazy Herdr endpoint that answers the reserved named locator as a Herdr server, and
/// one that drops the Herdr socket (or an explicit `--host-endpoint`) the environment names.
#[test]
fn lazy_host_endpoint_never_answers_the_reserved_locator() {
    let root = scratch("lazy-reserved");
    let reserved = PathBuf::from(crate::protocol::ids::NAMED_HOST_LOCATOR);
    let herdr_socket = root.join("herdr.sock");
    let base = InstanceInputs {
        home: Some(root.join("home")),
        non_pane_site: true,
        site_host: Some(reserved.clone()),
        ..InstanceInputs::default()
    };
    let with_env = InstanceInputs {
        env_host: Some(herdr_socket.clone()),
        ..base.clone()
    };
    assert_eq!(with_env.lazy_host_endpoint().unwrap(), herdr_socket);
    let flagged = InstanceInputs {
        host_flag: Some(root.join("flag.sock")),
        ..with_env
    };
    assert_eq!(
        flagged.lazy_host_endpoint().unwrap(),
        root.join("flag.sock")
    );
    // No Herdr anywhere: an error, never the reserved locator.
    let none = InstanceInputs {
        non_default_server: true,
        ..base
    };
    assert!(none.lazy_host_endpoint().is_err());
    fs::remove_dir_all(&root).unwrap();
}
