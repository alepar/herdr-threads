//! ht-kqrt.47 (H1f, E1 hc path): `HcHost::server_process` is the pid serving the core socket,
//! and the inherited-name check reads that process's environ. Run against the `fake-hc`
//! child, started through `test_support::spawn`, so the environ is a real process's.
use herdr_threads::{
    host::{
        hc::{HcClient, HcHost},
        named::ENV_SEAT_NAME,
    },
    ports::{HostCallContext, HostPort},
    protocol::time::{CallBudget, Cancellation, MonoInstant},
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

fn check(core: &Core, name: &str, hint: &str) -> Option<&'static str> {
    inherited_name_host(
        name,
        hint,
        std::slice::from_ref(&core.host),
        &context(),
        read_server_environ,
    )
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

    for hint in ["t1", "core-0001:t1"] {
        let label = check(&core, "x", hint).unwrap_or_else(|| panic!("{hint}: refused"));
        assert_eq!(label, "herdr-core");
    }
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
    assert_eq!(check(&core, "x", "t99"), None);
}

// Kills: a comparison on presence rather than value (any name refused), and a server without the
// variable refusing.
#[test]
fn per_command_name_in_terminal_honoured() {
    let carrying = core(Some("x"));
    assert_eq!(check(&carrying, "y", "t1"), None, "another name binds");

    let plain = core(None);
    assert_eq!(check(&plain, "x", "t1"), None, "no inherited name");
}

// Kills: a hint that ignores the terminal (the hc path never checked), a hint without a name
// (a plain shell has no inherited name to refuse), and a pane that loses to the terminal.
#[test]
fn terminal_site_sends_session_terminal_hint_beside_a_name() {
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
    assert_eq!(hint(&[&named, &terminal]).as_deref(), Some("core-a:t4"));
    assert_eq!(hint(&[&terminal]), None, "no name, no hint");
    assert_eq!(
        hint(&[&named, &terminal, &[("HERDR_PANE_ID", "w1:p2")]]).as_deref(),
        Some("w1:p2")
    );
}
