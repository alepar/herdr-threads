//! ht-kqrt.47 (H1f, E1 hc path): `HcHost::server_process` is the pid serving the core socket,
//! and the inherited-name check reads that process's environ. Run against the `fake-hc`
//! child, started through `test_support::spawn`, so the environ is a real process's.
use herdr_threads::{
    host::{
        hc::{HcClient, HcHost},
        named::ENV_SEAT_NAME,
    },
    ports::{HostCallContext, HostPort},
    protocol::{
        results::ApiError,
        time::{CallBudget, Cancellation, MonoInstant},
    },
    service::locate::{inherited_name_detail, inherited_name_host, read_server_environ},
    test_support::{
        scratch,
        spawn::{self, OwnedChild, SpawnOwned},
    },
};
use std::{process::Stdio, sync::Arc, time::Duration};

struct Core {
    child: OwnedChild,
    host: Arc<dyn HostPort>,
    /// The core's socket path, as a terminal hint names it.
    socket: String,
}

/// A `fake-hc` child whose environ carries `name` (or not) and an `HcHost` over its socket.
fn core(name: Option<&str>) -> Core {
    let socket = scratch::temp_path(format!("ht-hc-inherit-{}", uuid::Uuid::new_v4().simple()));
    let mut command = spawn::command(env!("CARGO_BIN_EXE_fake-hc"));
    command
        .arg("--socket")
        .arg(&socket)
        .env_remove(ENV_SEAT_NAME)
        .stdin(Stdio::null());
    if let Some(name) = name {
        command.env(ENV_SEAT_NAME, name);
    }
    let child = command.spawn_owned().expect("fake-hc starts");
    let host = HcHost::new(HcClient::new(&socket));
    for _ in 0..400 {
        if host.server_process().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Core {
        child,
        host: Arc::new(host),
        socket: socket.to_str().expect("utf-8 socket path").to_owned(),
    }
}

fn context() -> HostCallContext {
    HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(u64::MAX / 2),
            cancellation: Cancellation::default(),
        },
        expected_boot: None,
        expected_epoch: None,
    }
}

/// The inherited-name check with a `core` closure that answers only the sockets of `cores`.
fn check_cores(cores: &[&Core], name: &str, hint: &str) -> Option<&'static str> {
    let core = |socket: &str| {
        cores
            .iter()
            .find(|core| core.socket == socket)
            .map(|core| Arc::clone(&core.host))
            .ok_or_else(|| ApiError::host_unavailable("no such core"))
    };
    inherited_name_host(name, hint, &[], &core, &context(), read_server_environ)
}

fn check(core: &Core, name: &str, hint: &str) -> Option<&'static str> {
    check_cores(&[core], name, hint)
}

// Kills: a `server_process` left at the trait default (`None`), one that returns the test's own
// pid (the client's side of the connection) or a pid other than the core process.
#[test]
fn server_process_is_fake_hc_pid() {
    let core = core(None);
    assert_eq!(core.host.server_process(), Some(core.child.id()));
    assert_ne!(core.host.server_process(), Some(std::process::id()));
}

// Kills: no check on the hc path (the name binds), a check that reads the wrong process, a
// refusal that names Herdr instead of herdr-core, and a terminal hint sent to a Herdr-pane host.
#[test]
fn inherited_name_refused_m6_herdr_core() {
    let core = core(Some("x"));
    // The child's environ really carries the name (the check reads it, not a stub).
    let environ = read_server_environ(core.child.id()).expect("same-user child is readable");
    assert!(
        environ
            .iter()
            .any(|(k, v)| k.as_os_str() == ENV_SEAT_NAME && v.as_os_str() == "x")
    );

    let hint = format!("{}:t1", core.socket);
    let label = check(&core, "x", &hint).expect("refused");
    assert_eq!(label, "herdr-core");
    assert_eq!(
        inherited_name_detail("x", "herdr-core"),
        format!(
            "{ENV_SEAT_NAME}=x inherited from the herdr-core server; \
             use another name per command or restart the server without it"
        )
    );
    // A Herdr pane hint is not this core's to answer.
    assert_eq!(check(&core, "x", "w1:p1"), None);
    // A terminal the core does not hold is not asked about.
    assert_eq!(check(&core, "x", &format!("{}:t99", core.socket)), None);
    // A socket that is no attachable core skips the check.
    assert_eq!(check(&core, "x", "/no/such.sock:t1"), None);
}

// Kills: a comparison on presence rather than value (any name refused), and a server without the
// variable refusing.
#[test]
fn per_command_name_in_terminal_honoured() {
    let carrying = core(Some("x"));
    let hint = format!("{}:t1", carrying.socket);
    assert_eq!(check(&carrying, "y", &hint), None, "another name binds");

    let plain = core(None);
    let hint = format!("{}:t1", plain.socket);
    assert_eq!(check(&plain, "x", &hint), None, "no inherited name");
}

// Kills: a check that takes the first core holding `t1` instead of the hinted one: two cores own
// `t1`, only the second carries the name.
#[test]
fn inherited_name_checks_the_hinted_core_only() {
    let first = core(None);
    let second = core(Some("x"));
    let both = [&first, &second];
    assert_eq!(
        check_cores(&both, "x", &format!("{}:t1", first.socket)),
        None
    );
    assert_eq!(
        check_cores(&both, "x", &format!("{}:t1", second.socket)),
        Some("herdr-core")
    );
}

// Kills: a hint with a pane and a terminal checking only its first part.
#[test]
fn hint_with_pane_and_terminal_checks_both() {
    let carrying = core(Some("x"));
    let hint = format!("w1:p2\n{}:t1", carrying.socket);
    assert_eq!(check(&carrying, "x", &hint), Some("herdr-core"));
}

// Kills: a core that cannot be had refusing (or erroring) instead of skipping its candidate.
#[test]
fn unattachable_core_skips_the_check() {
    let carrying = core(Some("x"));
    let hint = format!("/gone/core.sock:t1\n{}:t1", carrying.socket);
    // The unattachable candidate is skipped; the attachable one still refuses.
    assert_eq!(check(&carrying, "x", &hint), Some("herdr-core"));
    assert_eq!(check_cores(&[], "x", "/gone/core.sock:t1"), None);
}

// Kills: a hint that ignores the terminal (the hc path never checked), a hint without a name
// (a plain shell has no inherited name to refuse), a pane that loses the terminal, or a terminal
// that loses its socket.
#[test]
fn named_caller_hint_carries_socket_and_every_candidate() {
    use herdr_threads::cli::caller_location::CallerEnv;
    fn env(vars: &[(&str, &str)]) -> CallerEnv {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        CallerEnv::from_lookup(|name| {
            vars.iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into())
        })
    }
    let terminal = [
        ("HC_TERMINAL_ID", "t4"),
        ("HC_TERMINAL_SOCKET", "/run/hc/core-a.sock"),
    ];
    let named = [(ENV_SEAT_NAME, "x")];
    let hint = |parts: &[&[(&str, &str)]]| env(&parts.concat()).locator_hint();
    assert_eq!(
        hint(&[&named, &terminal]).as_deref(),
        Some("/run/hc/core-a.sock:t4")
    );
    assert_eq!(hint(&[&terminal]), None, "no name, no hint");
    assert_eq!(
        hint(&[&named, &terminal, &[("HERDR_PANE_ID", "w1:p2")]]).as_deref(),
        Some("w1:p2\n/run/hc/core-a.sock:t4")
    );
}
