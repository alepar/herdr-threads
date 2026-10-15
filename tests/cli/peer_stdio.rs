//! ht-5fr.5.9: `herdr-threads peer --stdio`, the stateless ssh-side pump,
//! against a real store-only daemon it starts itself. Each test runs the
//! installed executable under an isolated state root and stops its daemon.
use herdr_threads::test_support::{
    isolation::TestIsolation,
    spawn::{OwnedChild, SpawnOwned},
};
use std::{
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const PREAMBLE: &[u8] = b"HTPEER1\n";
const WAIT: Duration = Duration::from_secs(20);

fn pump(iso: &TestIsolation, state: &Path) -> OwnedChild {
    iso.command(BIN)
        .arg("--state-dir")
        .arg(state)
        .args(["peer", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_owned()
        .unwrap()
}

fn wait_exit(child: &mut OwnedChild) -> std::process::ExitStatus {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "pump did not exit within {WAIT:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read_preamble(child: &mut OwnedChild) {
    let mut first = [0u8; 8];
    child
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut first)
        .unwrap();
    assert_eq!(&first[..], PREAMBLE);
}

/// `daemon stop` for the store-only daemon a pump started. The CLI still
/// wants a host endpoint for the call; `stop` never contacts it, so a path
/// that names no socket serves.
fn daemon_stop(iso: &TestIsolation, state: &Path) -> std::process::Output {
    iso.command(BIN)
        .arg("--state-dir")
        .arg(state)
        .arg("--host-endpoint")
        .arg(iso.path("no-host.sock"))
        .args(["daemon", "stop"])
        .output()
        .unwrap()
}

/// Stops the daemon a pump started, whatever the test outcome.
struct StopDaemon<'a>(&'a TestIsolation, &'a Path);
impl Drop for StopDaemon<'_> {
    fn drop(&mut self) {
        daemon_stop(self.0, self.1);
    }
}

#[test]
fn pump_prints_preamble_first() {
    let iso = TestIsolation::new("peer-stdio-preamble");
    let state = iso.path("state");
    let _stop = StopDaemon(&iso, &state);
    let mut child = pump(&iso, &state);
    read_preamble(&mut child);
}

#[test]
fn stdin_eof_exits_zero() {
    let iso = TestIsolation::new("peer-stdio-eof");
    let state = iso.path("state");
    let _stop = StopDaemon(&iso, &state);
    let mut child = pump(&iso, &state);
    read_preamble(&mut child);
    drop(child.stdin.take());
    assert_eq!(wait_exit(&mut child).code(), Some(0));
    let mut rest = Vec::new();
    child
        .stdout
        .as_mut()
        .unwrap()
        .read_to_end(&mut rest)
        .unwrap();
    assert!(rest.is_empty(), "nothing follows the preamble: {rest:?}");
}

#[test]
fn daemon_stop_makes_pump_exit() {
    let iso = TestIsolation::new("peer-stdio-stop");
    let state = iso.path("state");
    let _stop = StopDaemon(&iso, &state);
    let mut child = pump(&iso, &state);
    read_preamble(&mut child);
    // stdin stays open: only the daemon going away ends the pump.
    let out = daemon_stop(&iso, &state);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(wait_exit(&mut child).code(), Some(0));
}

#[test]
fn no_daemon_startable_exits_3_no_stdout() {
    let iso = TestIsolation::new("peer-stdio-unsafe");
    let state = iso.path("state");
    // A group- and world-readable node directory is refused by the daemon.
    let node = state.join("node");
    fs::create_dir_all(&node).unwrap();
    fs::set_permissions(&node, fs::Permissions::from_mode(0o755)).unwrap();
    let out = pump(&iso, &state).wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(out.stdout.is_empty(), "stdout bytes: {:?}", out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.starts_with("herdr-threads peer: daemon unavailable: "),
        "{stderr}"
    );
    assert_eq!(stderr.trim_end().lines().count(), 1, "{stderr}");
}
