//! The launch-gate shim: env blanking, the bounded authorization wait and `exec` keeping the pid.
//!
//! `exec` replaces the process, so the exec tests re-run this test binary on
//! `gate_child_entry` (marked by `HT_GATE_CHILD`), which runs the shim against a fake daemon and
//! execs a `sh` stand-in that records its env and pid under `HT_GATE_OUT`.

use super::*;
use crate::{
    cli::caller_location::{CallerLocator, EnvMode, Located},
    ports::HostKind,
    protocol::{
        handoff::{LaunchGateRefusal, LaunchGateReply},
        output::OutputSpec,
    },
    test_support::spawn::SpawnOwned,
};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

const CHILD: &str = "HT_GATE_CHILD";
const OUT: &str = "HT_GATE_OUT";

/// A daemon that answers every authorization with `reply`, recording the request.
struct FakeDaemon {
    reply: Result<LaunchGateReply, ApiError>,
    delay: Duration,
    seen: Mutex<Vec<AuthorizeLaunchGate>>,
    replied_marker: Option<PathBuf>,
}
impl LocalClient for FakeDaemon {
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        let Command::AuthorizeLaunchGate(request) = command else {
            panic!("the shim sends only AuthorizeLaunchGate");
        };
        self.seen.lock().unwrap().push(*request);
        std::thread::sleep(self.delay);
        if let Some(marker) = &self.replied_marker {
            fs::write(marker, "").unwrap();
        }
        self.reply.clone().map(CommandResult::LaunchGate)
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

fn fake(reply: LaunchGateReply) -> FakeDaemon {
    FakeDaemon {
        reply: Ok(reply),
        delay: Duration::ZERO,
        seen: Mutex::new(Vec::new()),
        replied_marker: None,
    }
}

fn gate() -> Gate {
    Gate {
        compound: OperationId::new("op-1"),
        attempt: BootstrapAttempt::new(2).unwrap(),
        socket: "/run/core.sock".into(),
        terminal: "t7".into(),
    }
}

fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
    let owned: BTreeMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect();
    move |name| owned.get(name).map(OsString::from)
}

#[test]
fn bootstrap_parses_compound_and_attempt() {
    let (compound, attempt) = parse_bootstrap("op:with:colons:3").unwrap();
    assert_eq!(compound.as_str(), "op:with:colons");
    assert_eq!(attempt.get(), 3);
    for bad in ["op", "op:0", "op:x", ":1", "op:-1"] {
        assert!(parse_bootstrap(bad).is_err(), "{bad}");
    }
}

#[test]
fn authorized_reply_releases_and_carries_the_witness() {
    let daemon = fake(LaunchGateReply::Authorized);
    let clock = crate::app::SystemClock::new();
    authorize(&daemon, &gate(), "core-a", 5_000, &clock).unwrap();
    let seen = daemon.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].core_id, "core-a");
    assert_eq!(seen[0].terminal, "t7");
    assert_eq!(seen[0].compound.as_str(), "op-1");
    assert_eq!(seen[0].attempt.get(), 2);
    assert_eq!(seen[0].timeout_ms, 5_000);
}

#[test]
fn authorize_sends_the_core_socket() {
    let daemon = fake(LaunchGateReply::Authorized);
    let clock = crate::app::SystemClock::new();
    authorize(&daemon, &gate(), "core-a", 5_000, &clock).unwrap();
    let seen = daemon.seen.lock().unwrap();
    assert_eq!(seen[0].socket.as_deref(), Some("/run/core.sock"));
}

#[test]
fn every_refusal_is_an_error() {
    let clock = crate::app::SystemClock::new();
    for reason in [
        LaunchGateRefusal::UnknownBootstrap,
        LaunchGateRefusal::WitnessMismatch,
        LaunchGateRefusal::StaleAttempt,
        LaunchGateRefusal::TimedOut,
    ] {
        let daemon = fake(LaunchGateReply::Refused { reason });
        let error = authorize(&daemon, &gate(), "core-a", 5_000, &clock).unwrap_err();
        let RunError::Api(api) = error else {
            panic!("refusal is an api error");
        };
        let timed_out = reason == LaunchGateRefusal::TimedOut;
        assert_eq!(
            api.code == ErrorCode::DeadlineExceeded,
            timed_out,
            "{reason:?}"
        );
    }
}

#[test]
fn out_of_range_timeout_is_refused_before_any_call() {
    let daemon = fake(LaunchGateReply::Authorized);
    let clock = crate::app::SystemClock::new();
    assert!(authorize(&daemon, &gate(), "core-a", 0, &clock).is_err());
    assert!(daemon.seen.lock().unwrap().is_empty());
}

#[test]
fn gate_comes_from_the_terminal_env_even_beside_a_leaked_seat_name() {
    let env = terminal_env(lookup(&[
        ("HC_TERMINAL_ID", "t9"),
        ("HC_TERMINAL_SOCKET", "/run/c.sock"),
        ("HERDR_THREADS_SEAT_NAME", "leaked"),
        ("HERDR_PANE_ID", "p_1"),
    ]));
    let gate = gate_from_env("op-1:1", &env).unwrap();
    assert_eq!(
        (gate.terminal.as_str(), gate.socket.as_str()),
        ("t9", "/run/c.sock")
    );
    assert!(gate_from_env("op-1:1", &terminal_env(lookup(&[]))).is_err());
}

#[test]
fn scrub_names_cover_herdr_adapters_and_gate_list_but_keep_hc_terminal_vars() {
    let names = scrub_names([
        OsString::from("HERDR_SOMETHING_NEW"),
        OsString::from("PATH"),
    ]);
    for expected in [
        "HERDR_SOMETHING_NEW",
        "HERDR_PANE_ID",
        "HERDR_THREADS_SEAT_NAME",
        "HERDR_THREADS_SEAT_READONLY",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_PID",
        "CODEX_SESSION_ID",
    ] {
        assert!(names.contains(&OsString::from(expected)), "{expected}");
    }
    for kept in ["HC_TERMINAL_ID", "HC_TERMINAL_SOCKET", "HC_SCOPE", "PATH"] {
        assert!(!names.contains(&OsString::from(kept)), "{kept}");
    }
}

#[test]
fn host_launch_argv_codex_hc_has_no_daemon() {
    let registry = crate::harness::registry::builtins();
    let codex = registry
        .registrations()
        .iter()
        .find(|r| r.metadata().id == "codex")
        .unwrap();
    assert!(
        codex
            .host_launch_argv(HostKind::Hc)
            .contains(&"--no-daemon".to_owned())
    );
    assert!(codex.host_launch_argv(HostKind::Named).is_empty());
}

#[test]
fn host_launch_argv_herdr_empty_for_every_harness() {
    let registry = crate::harness::registry::builtins();
    assert!(!registry.registrations().is_empty());
    for registration in registry.registrations() {
        assert!(
            registration.host_launch_argv(HostKind::Herdr).is_empty(),
            "{}",
            registration.metadata().id
        );
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = crate::test_support::scratch::temp_path(format!(
        "ht-lg-{}-{tag}",
        &uuid::Uuid::new_v4().to_string()[..8]
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The re-run entry: only does anything in a child marked by `HT_GATE_CHILD`.
#[test]
fn gate_child_entry() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    let out = PathBuf::from(std::env::var(OUT).unwrap());
    fs::write(out.join("shim_pid"), std::process::id().to_string()).unwrap();
    let script = "env > \"$1/env\"; echo $$ > \"$1/exec_pid\"; \
                  if [ -f \"$1/replied\" ]; then echo yes > \"$1/after_reply\"; fi";
    let argv: Vec<String> = ["/bin/sh", "-c", script, "sh", out.to_str().unwrap()]
        .map(String::from)
        .into();
    let roots = vec!["claude=/gate/claude-root".to_owned()];
    let clock = crate::app::SystemClock::new();
    let env = terminal_env(|name| std::env::var_os(name));
    let ambient: Vec<OsString> = std::env::vars_os().map(|(k, _)| k).collect();
    let error = match mode.as_str() {
        "silent" => {
            let socket = out.join("d.sock");
            let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
            let held = listener.try_clone().unwrap();
            std::thread::spawn(move || {
                let mut streams = Vec::new();
                while let Ok((stream, _)) = held.accept() {
                    streams.push(stream);
                }
            });
            let client = crate::client::local::LocalSocketClient::new(
                socket,
                std::sync::Arc::new(crate::app::SystemClock::new()),
                crate::node::NodeUuid::new_v4(),
                Some(uuid::Uuid::new_v4()),
            );
            run_with(
                &client,
                &clock,
                &env,
                "op-1:1",
                &roots,
                &argv,
                ambient,
                1,
                |_| Ok("core-a".into()),
            )
        }
        _ => {
            let mut daemon = fake(LaunchGateReply::Authorized);
            daemon.delay = Duration::from_millis(300);
            daemon.replied_marker = Some(out.join("replied"));
            run_with(
                &daemon,
                &clock,
                &env,
                "op-1:1",
                &roots,
                &argv,
                ambient,
                5_000,
                |_| Ok("core-a".into()),
            )
        }
    };
    // Reached only when `exec` did not happen.
    fs::write(out.join("not_exec"), format!("{error:?}")).unwrap();
    std::process::exit(7);
}

struct GateRun {
    out: PathBuf,
    status: std::process::ExitStatus,
    elapsed: Duration,
}

fn run_gate(mode: &str, tag: &str, leaked: &[(&str, &str)]) -> GateRun {
    let out = scratch(tag);
    let mut command = crate::test_support::spawn::command(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!(
                "{}::gate_child_entry",
                module_path!().split_once("::").map_or("", |(_, rest)| rest)
            ),
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, mode)
        .env(OUT, &out)
        .env("HC_TERMINAL_ID", "t7")
        .env("HC_TERMINAL_SOCKET", "/run/core.sock")
        .env("HC_SCOPE", "scope-1");
    for (k, v) in leaked {
        command.env(k, v);
    }
    let started = Instant::now();
    let output = command.spawn_owned().unwrap().wait_with_output().unwrap();
    GateRun {
        out,
        status: output.status,
        elapsed: started.elapsed(),
    }
}

const LEAKED: &[(&str, &str)] = &[
    ("HERDR_PANE_ID", "p_leak"),
    ("HERDR_TAB_ID", "tab_leak"),
    ("HERDR_SOCKET_PATH", "/leak/herdr.sock"),
    ("HERDR_THREADS_SEAT_NAME", "leaked-name"),
    ("HERDR_THREADS_SEAT_READONLY", "1"),
    ("CLAUDE_CONFIG_DIR", "/leaked/claude"),
    ("CLAUDE_CODE_SESSION_ID", "leaked-session"),
    ("CLAUDE_PID", "4242"),
    ("CODEX_SESSION_ID", "leaked-codex"),
];

fn exec_env(run: &GateRun) -> BTreeMap<String, String> {
    fs::read_to_string(run.out.join("env"))
        .unwrap_or_else(|_| {
            panic!(
                "stand-in never ran: {:?}",
                fs::read_to_string(run.out.join("not_exec"))
            )
        })
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

#[test]
fn exec_env_has_no_herdr_name_or_adapter_session_vars() {
    let run = run_gate("authorize", "env", LEAKED);
    let env = exec_env(&run);
    for (name, _) in LEAKED.iter().filter(|(n, _)| *n != "CLAUDE_CONFIG_DIR") {
        assert!(!env.contains_key(*name), "{name} survived the shim");
    }
    assert!(env.keys().all(|k| !k.starts_with("HERDR_")));
    assert!(!run.out.join("not_exec").exists());
    fs::remove_dir_all(&run.out).unwrap();
}

#[test]
fn leaked_claude_config_dir_cannot_win() {
    let run = run_gate("authorize", "cfg", LEAKED);
    let env = exec_env(&run);
    assert_eq!(env["CLAUDE_CONFIG_DIR"], "/gate/claude-root");
    fs::remove_dir_all(&run.out).unwrap();
}

// Kills: an adapter that leaves its config-root variable out of `child_scrub_vars`, so an
// autostarted core's root reaches a launch whose plan carries no `--config-root` (a peer
// launch). `HOME` stays: core and caller share one user's home.
#[test]
fn prepare_removes_every_adapter_root_variable() {
    let ambient = ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "HERMES_HOME", "HOME"].map(OsString::from);
    let command = prepare(&["/bin/true".to_owned()], &[], ambient).unwrap();
    let removed: Vec<_> = command
        .get_envs()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| name.to_owned())
        .collect();
    for root in ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "HERMES_HOME"] {
        assert!(removed.contains(&OsString::from(root)), "{root}");
    }
    assert!(!removed.contains(&OsString::from("HOME")));
}

#[test]
fn keeps_hc_terminal_id_socket_and_scope() {
    let run = run_gate("authorize", "hc", LEAKED);
    let env = exec_env(&run);
    assert_eq!(env["HC_TERMINAL_ID"], "t7");
    assert_eq!(env["HC_TERMINAL_SOCKET"], "/run/core.sock");
    assert_eq!(env["HC_SCOPE"], "scope-1");
    fs::remove_dir_all(&run.out).unwrap();
}

#[test]
fn waits_for_core_tn_authorization_then_execs() {
    let run = run_gate("authorize", "wait", LEAKED);
    assert!(run.out.join("replied").exists());
    // The stand-in saw the daemon's reply already on disk when it started.
    assert_eq!(
        fs::read_to_string(run.out.join("after_reply"))
            .unwrap()
            .trim(),
        "yes"
    );
    assert!(run.elapsed >= Duration::from_millis(300));
    fs::remove_dir_all(&run.out).unwrap();
}

#[test]
fn times_out_bounded_without_exec() {
    let run = run_gate("silent", "silent", LEAKED);
    assert!(
        !run.out.join("env").exists(),
        "the harness ran without authorization"
    );
    assert!(run.out.join("not_exec").exists());
    assert_eq!(run.status.code(), Some(7));
    // 1 ms wait + 2 s transport slack, not the silent daemon's forever.
    assert!(run.elapsed < Duration::from_secs(20), "{:?}", run.elapsed);
    fs::remove_dir_all(&run.out).unwrap();
}

#[test]
fn exec_keeps_pid() {
    let run = run_gate("authorize", "pid", LEAKED);
    let shim = fs::read_to_string(run.out.join("shim_pid")).unwrap();
    let exec = fs::read_to_string(run.out.join("exec_pid")).unwrap();
    assert_eq!(shim.trim(), exec.trim());
    fs::remove_dir_all(&run.out).unwrap();
}

#[test]
fn exec_child_resolves_terminal_site() {
    let run = run_gate("authorize", "site", LEAKED);
    let env = exec_env(&run);
    let seen = crate::cli::caller_location::CallerEnv::from_lookup(|name| {
        env.get(name).map(OsString::from)
    });
    for mode in [EnvMode::Cli, EnvMode::Hook] {
        assert_eq!(
            seen.locate(mode).unwrap(),
            Some(Located::One(CallerLocator::Terminal {
                socket: "/run/core.sock".into(),
                id: "t7".into(),
            })),
            "rule 2, not rule 4's no-family error"
        );
    }
    fs::remove_dir_all(&run.out).unwrap();
}
