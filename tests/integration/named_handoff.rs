//! ht-kqrt.135 (N5-cli): a process that carries `HERDR_THREADS_SEAT_NAME` and no Herdr pane runs
//! the real `handoff` CLI into a pane of an isolated Herdr server (the Herdr endpoint resolves
//! lazily from the environment), and a name inherited from the server's own environment is
//! refused with M6 on the CLI path.

use super::herdr_release_smoke::{Matrix, json, wait_for};
use herdr_threads::test_support::{
    isolated_herdr::IsolatedHerdr,
    isolation::{TestIsolation, as_agent},
    resolve_seeded,
    spawn::{OwnedChild, SpawnOwned},
    spawn_node_daemon,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

fn herdr_present() -> bool {
    Command::new("herdr").arg("--version").output().is_ok()
}

/// `command` run to completion with `stdin` fed to it: (exit code, stdout, stderr).
fn run(mut command: Command, stdin: Option<&[u8]>) -> (Option<i32>, String, String) {
    use std::io::Write;
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child: OwnedChild = command.spawn_owned().expect("spawn the process");
    if let Some(bytes) = stdin {
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    let output = child.wait_with_output().expect("process output");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The named process: the name, a harness session, the private Herdr's socket and no pane.
fn named(m: &Matrix, name: &str, session: &str) -> Command {
    let mut command = m.cli();
    command.env("HERDR_THREADS_SEAT_NAME", name);
    as_agent(&mut command, "claude", session);
    command
}

fn state_dir(m: &Matrix) -> PathBuf {
    m.host.root().join("st/herdr/plugins/herdr-threads")
}

/// The SessionStart hook of `name` until it enrolls (the named host's first observation of a new
/// seat lands on the daemon's observation cadence).
fn enroll(m: &Matrix, name: &str, session: &str) {
    let payload = format!(
        r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"startup"}}"#
    );
    wait_for(&format!("{name} enrolled"), Duration::from_secs(60), || {
        let mut command = named(m, name, session);
        command
            .arg("--state-dir")
            .arg(state_dir(m))
            .args(["hook", "claude"]);
        let (_, stdout, stderr) = run(command, Some(payload.as_bytes()));
        if stdout.contains("Your seat") {
            Ok(())
        } else {
            Err(format!("{stdout} {stderr}"))
        }
    });
}

// Kills: a named caller that cannot reach `handoff` (no pane), a handoff that dials the reserved
// locator as a Herdr server or never resolves the Herdr endpoint lazily, a recipient seat
// resolved on the named host, a daemon that refuses the named claim against the Herdr namespace,
// and a frozen claim that is not the reserved locator.
#[test]
fn handoff_from_named_seat_into_isolated_herdr_pane() {
    if !herdr_present() {
        println!("skipped: no herdr: handoff_from_named_seat_into_isolated_herdr_pane");
        return;
    }
    let m = Matrix::new("named-handoff");
    m.setup("codex");
    let target = m.workspace("named-target");
    enroll(&m, "alice", "s-alice");

    // The state directory is given; the Herdr endpoint is not: it comes from HERDR_SOCKET_PATH,
    // resolved at this handoff only.
    let mut command = named(&m, "alice", "s-alice");
    command
        .arg("--json")
        .arg("--state-dir")
        .arg(state_dir(&m))
        .args([
            "handoff",
            "--pane",
            &target,
            "--kind",
            "codex",
            "--",
            "named-task",
        ]);
    let (code, stdout, stderr) = run(command, None);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let out: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let frame = out
        .get("bootstrap")
        .or_else(|| out.get("handoff"))
        .unwrap_or(&out);
    assert_eq!(frame["outcome"], "started", "{out}");
    let pane = frame["pane"]
        .as_str()
        .or_else(|| frame["launch"]["pane"].as_str())
        .unwrap_or_else(|| panic!("{out}"));
    assert_eq!(pane, target, "{out}");
    let thread = frame["thread"].as_str().unwrap_or_else(|| panic!("{out}"));

    // The task arrived through the recipient's own hook.
    wait_for("the task reaches the pane", Duration::from_secs(30), || {
        let log = m.log_of(&target);
        if log.iter().any(|e| {
            e["kind"] == "hook" && e["context"].as_str().is_some_and(|c| c.contains(thread))
        }) {
            Ok(())
        } else {
            Err(format!("{log:?}"))
        }
    });

    // The recipient's seat is on the Herdr host; the caller's claim stays at the named host.
    let paths = resolve_seeded(&state_dir(&m));
    let db = rusqlite::Connection::open(paths.database_path).unwrap();
    let kind: String = db
        .query_row(
            "SELECT h.kind FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id \
             WHERE s.target_id=?1 AND s.state<>'retired'",
            [&target],
            |r| r.get(0),
        )
        .expect("a recipient seat on the pane");
    assert_ne!(kind, "named", "the recipient lives on the Herdr host");
    let mut frozen_at_named_host = false;
    for entry in fs::read_dir(state_dir(&m).join("node/intents"))
        .unwrap()
        .flatten()
    {
        if let Ok(text) = fs::read_to_string(entry.path()) {
            frozen_at_named_host |= text.contains(herdr_threads::protocol::ids::NAMED_HOST_LOCATOR);
        }
    }
    assert!(
        frozen_at_named_host,
        "the frozen claim host is the reserved named locator"
    );
    drop(db);
}

/// `launch --kind codex` into `pane` with the stand-in, from the operator side (the endpoint is
/// given): the peer seat a named caller then hands off to.
fn launch_peer(m: &Matrix, pane: &str) -> String {
    wait_for("peer launched", Duration::from_secs(30), || {
        let mut command = m.cli();
        m.globals(&mut command);
        command
            .args([
                "launch",
                "--kind",
                "codex",
                "--pane",
                pane,
                "--harness-binary",
            ])
            .arg(m.bin.join("codex"));
        let (code, stdout, stderr) = run(command, None);
        if code != Some(0) {
            return Err(format!("{stdout} {stderr}"));
        }
        let out: serde_json::Value = serde_json::from_str(&stdout).map_err(|e| e.to_string())?;
        out["launch"]["seat"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| stdout.clone())
    })
}

/// A named caller `alice` and a live pane seat; `handoff --existing --seat PEER` as alice, with
/// `extra` after the verb. (code, stdout, stderr).
fn named_existing(m: &Matrix, peer: &str, body: &str) -> (Option<i32>, String, String) {
    let mut command = named(m, "alice", "s-alice");
    command
        .arg("--json")
        .arg("--state-dir")
        .arg(state_dir(m))
        .args(["handoff", "--existing", "--seat", peer, "--", body]);
    run(command, None)
}

// Kills: a `handoff --existing` that dials the reserved named locator as a Herdr server (it
// always takes the topology runtime, not `handoff::run`), a frozen namespace or output context
// carrying the locator instead of the lazily resolved endpoint, and a frozen claim that is not
// the reserved locator.
#[test]
fn handoff_existing_seat_from_named_seat() {
    if !herdr_present() {
        println!("skipped: no herdr: handoff_existing_seat_from_named_seat");
        return;
    }
    let m = Matrix::new("named-existing");
    m.setup("codex");
    let peer_pane = m.workspace("named-existing-peer");
    let peer = launch_peer(&m, &peer_pane);
    enroll(&m, "alice", "s-alice");

    let (code, stdout, stderr) = named_existing(&m, &peer, "existing-task");
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains(&peer), "{stdout}");
    assert!(
        !stdout.contains(herdr_threads::protocol::ids::NAMED_HOST_LOCATOR),
        "the Herdr endpoint is not the reserved locator: {stdout}"
    );
    let mut frozen_at_named_host = false;
    for entry in fs::read_dir(state_dir(&m).join("node/intents"))
        .unwrap()
        .flatten()
    {
        if let Ok(text) = fs::read_to_string(entry.path()) {
            frozen_at_named_host |= text.contains(herdr_threads::protocol::ids::NAMED_HOST_LOCATOR);
        }
    }
    assert!(
        frozen_at_named_host,
        "the frozen claim host is the reserved named locator"
    );
}

fn find_key<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|v| find_key(v, key))),
        serde_json::Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
        _ => None,
    }
}

// Kills: a retry of a named caller's existing-seat handoff that dials the reserved locator (or
// compares the frozen Herdr endpoint with it) and so cannot replay the recorded handoff.
#[test]
fn retry_of_a_named_existing_handoff_runs() {
    if !herdr_present() {
        println!("skipped: no herdr: retry_of_a_named_existing_handoff_runs");
        return;
    }
    let m = Matrix::new("named-existing-retry");
    m.setup("codex");
    let peer_pane = m.workspace("named-retry-peer");
    let peer = launch_peer(&m, &peer_pane);
    enroll(&m, "alice", "s-alice");

    let (code, stdout, stderr) = named_existing(&m, &peer, "retry-task");
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let out: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let reference = find_key(&out, "recovery_ref")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("a recovery reference: {out}"))
        .to_owned();

    let mut command = named(&m, "alice", "s-alice");
    command
        .arg("--json")
        .arg("--state-dir")
        .arg(state_dir(&m))
        .args(["retry", &reference]);
    let (code, stdout, stderr) = run(command, None);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains(&peer), "{stdout}");
}

fn real_herdr() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap();
    std::env::split_paths(&path)
        .map(|dir| dir.join("herdr"))
        .find(|candidate| candidate.is_file())
        .expect("herdr on PATH")
}

// Kills: a CLI path that binds a name the Herdr server's own environment carries instead of
// refusing it with M6 (the daemon's ResolveSeat guard runs on the CLI path too).
#[test]
fn name_in_the_server_environ_is_refused_with_m6_on_the_cli_path() {
    if !herdr_present() {
        println!(
            "skipped: no herdr: name_in_the_server_environ_is_refused_with_m6_on_the_cli_path"
        );
        return;
    }
    let host = IsolatedHerdr::new("named-m6").expect("herdr present");
    // A `herdr` that exports the name before it becomes the server: the server's environment (and
    // so every pane's) carries it.
    let wrapper_dir = host.root().join("wrap");
    fs::create_dir_all(&wrapper_dir).unwrap();
    let wrapper = wrapper_dir.join("herdr");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexport HERDR_THREADS_SEAT_NAME=reviewer\nexec '{}' \"$@\"\n",
            real_herdr().display()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    host.start_with_path_prefix(&wrapper_dir);
    let created = {
        let out = host
            .command(real_herdr())
            .args(["workspace", "create", "--label", "m6", "--no-focus"])
            .output()
            .unwrap();
        json(&out, "workspace create")
    };
    let pane = created["result"]["root_pane"]["pane_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let isolation = TestIsolation::new("named-m6");
    let state = isolation.path("state");
    fs::create_dir_all(&state).unwrap();
    let state = state.canonicalize().unwrap();
    resolve_seeded(&state);
    let mut daemon = spawn_node_daemon(
        &isolation,
        BIN,
        &state,
        Some(&host.socket_path()),
        &isolation.path("daemon.stderr"),
    )
    .expect("spawn the daemon");
    let started = Instant::now();
    let call = |args: &[&str]| {
        let mut command = host.command(BIN);
        command.arg("--state-dir").arg(&state).args(args);
        command.env("PATH", std::env::var_os("PATH").unwrap());
        command
    };
    while run(call(&["daemon", "health"]), None).0 != Some(0) {
        assert!(started.elapsed() < Duration::from_secs(30), "daemon up");
        std::thread::sleep(Duration::from_millis(100));
    }

    let mut command = call(&["inbox"]);
    command
        .env("HERDR_THREADS_SEAT_NAME", "reviewer")
        .env("HERDR_PANE_ID", &pane);
    as_agent(&mut command, "claude", "s-m6");
    let (code, _, stderr) = run(command, None);
    assert_ne!(code, Some(0), "{stderr}");
    let golden = include_str!("../cli/golden/m6.txt").trim_end();
    assert!(stderr.contains(golden), "{stderr}");

    let _ = run(call(&["daemon", "stop"]), None);
    let _ = daemon.kill();
    drop(isolation);
}
