//! ht-5fr.5.9: `herdr-threads peer --stdio`, the stateless ssh-side pump,
//! against a real store-only daemon it starts itself. Each test runs the
//! installed executable under an isolated state root and stops its daemon.
use herdr_threads::node::NodeIdentity;
use herdr_threads::sync::frame::Role;
use herdr_threads::sync::link::{LocalHello, handshake, read_preamble as await_preamble};
use herdr_threads::test_support::{
    isolation::TestIsolation,
    spawn::{OwnedChild, SpawnOwned},
};
use std::sync::Arc;
use std::{
    fs,
    io::Read,
    os::{fd::AsFd, unix::fs::PermissionsExt},
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const PREAMBLE: &[u8] = b"HTPEER1\n";
const WAIT: Duration = Duration::from_secs(20);

/// `peer --stdio` with piped stdio and no Herdr host: `PATH` holds no `herdr`,
/// so the host deterministically resolves to none and a developer's Herdr is
/// never queried.
fn pump_cmd(iso: &TestIsolation) -> std::process::Command {
    let mut command = iso.command(BIN);
    command
        .env("PATH", "/usr/bin:/bin")
        .args(["peer", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn pump(iso: &TestIsolation, state: &Path) -> OwnedChild {
    let mut command = iso.command(BIN);
    command
        .env("PATH", "/usr/bin:/bin")
        .arg("--state-dir")
        .arg(state)
        .args(["peer", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn_owned().unwrap()
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

/// `daemon stop` by state dir alone; a store-only node needs no host.
fn daemon_stop(iso: &TestIsolation, state: &Path) -> std::process::Output {
    iso.command(BIN)
        .env("PATH", "/usr/bin:/bin")
        .arg("--state-dir")
        .arg(state)
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

/// The uuid of the key the daemon under `state` runs with.
fn daemon_node(state: &Path) -> herdr_threads::node::NodeUuid {
    let seed: [u8; 32] = fs::read(state.join("node").join("node.key"))
        .unwrap()
        .try_into()
        .unwrap();
    NodeIdentity::from_seed(seed).uuid()
}

/// Runs a dialer-role handshake over the pump's stdio with a test identity and
/// returns the peer it met; the link is dropped (stdin EOF) before it returns.
/// The returned file keeps the pump's stdout open: dropping it is a dead
/// channel (EPIPE, exit 1 by design), not a stdin EOF.
fn dial_through(child: &mut OwnedChild) -> (herdr_threads::sync::link::PeerInfo, std::fs::File) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    let keep = std::fs::File::from(
        child
            .stdout
            .as_ref()
            .unwrap()
            .as_fd()
            .try_clone_to_owned()
            .unwrap(),
    );
    let stdout = tokio::process::ChildStdout::from_std(child.stdout.take().unwrap()).unwrap();
    let stdin = tokio::process::ChildStdin::from_std(child.stdin.take().unwrap()).unwrap();
    let peer = runtime.block_on(async {
        let mut io = tokio::io::join(stdout, stdin);
        let deadline = tokio::time::Instant::now() + WAIT;
        let noise = await_preamble(&mut io, deadline).await.unwrap();
        assert_eq!(noise, 0, "nothing precedes the preamble");
        let me = LocalHello::new(
            Arc::new(NodeIdentity::from_seed([7; 32])),
            uuid::Uuid::new_v4(),
        );
        let established = handshake(io, &me, Role::Dialer, None, deadline)
            .await
            .unwrap();
        established.peer
    });
    (peer, keep)
}

/// Reads `file` to EOF. Tokio put the shared pipe description in
/// non-blocking mode, so `WouldBlock` is polled through, bounded by `WAIT`.
fn drain_to_eof(file: &mut std::fs::File) {
    let deadline = Instant::now() + WAIT;
    let mut buf = [0u8; 4096];
    loop {
        match file.read(&mut buf) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "pump stdout never reached EOF");
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("reading the pump's stdout: {error}"),
        }
    }
}

fn daemon_log(state: &Path) -> String {
    fs::read_to_string(state.join("node").join("daemon.log")).unwrap_or_default()
}

fn wait_for_log(state: &Path, needle: &str) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        let log = daemon_log(state);
        if log.contains(needle) {
            return log;
        }
        assert!(
            Instant::now() < deadline,
            "daemon log never said {needle:?}:\n{log}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn dialer_role_link_over_pump_completes_handshake() {
    let iso = TestIsolation::new("peer-stdio-handshake");
    let state = iso.path("state");
    let _stop = StopDaemon(&iso, &state);
    let mut child = pump(&iso, &state);
    let (peer, _stdout) = dial_through(&mut child);
    assert_eq!(
        peer.role,
        Role::Dialed,
        "the daemon answers as the dialed side"
    );
    assert_eq!(peer.node, daemon_node(&state));
    wait_for_log(&state, "sync: link up ");
}

#[test]
fn stdin_eof_logs_link_down() {
    let iso = TestIsolation::new("peer-stdio-link-down");
    let state = iso.path("state");
    let _stop = StopDaemon(&iso, &state);
    let mut child = pump(&iso, &state);
    // `dial_through` drops the pump's stdin when it returns: the pump shuts
    // the socket's write side and the daemon's link ends. Its stdout stays
    // open (`stdout`), so this is a clean stdin EOF, not a dead channel.
    let (peer, mut stdout) = dial_through(&mut child);
    let node = peer.node.to_string();
    let log = wait_for_log(&state, "sync: link down ");
    // The link is the test identity's, not the daemon's own.
    let ours = NodeIdentity::from_seed([7; 32]).uuid().to_string();
    assert!(
        log.contains(&format!("sync: link up {}", &ours[..8])),
        "{log}"
    );
    assert_ne!(node, ours);
    // Drain to EOF before waiting: the pump closes fd 1 on exit, and it can
    // never block on a full pipe.
    drain_to_eof(&mut stdout);
    assert_eq!(wait_exit(&mut child).code(), Some(0));
}

#[test]
fn pump_with_env_state_dir_and_no_host_links() {
    let iso = TestIsolation::new("peer-stdio-env-state");
    let state = iso.path("state");
    let _stop = StopDaemon(&iso, &state);
    let mut child = pump_cmd(&iso)
        .env("HERDR_PLUGIN_STATE_DIR", &state)
        .spawn_owned()
        .unwrap();
    let (peer, _stdout) = dial_through(&mut child);
    assert_eq!(peer.node, daemon_node(&state));
    wait_for_log(&state, "sync: link up ");
}

/// The ssh default: no flag, no plugin env, no host. The pump finds the
/// default plugin state dir (used when it exists).
#[test]
fn pump_with_no_state_flag_or_env_uses_the_default_state_dir() {
    let iso = TestIsolation::new("peer-stdio-default-state");
    let state = iso.path("xdg/state").join("herdr/plugins/herdr-threads");
    fs::create_dir_all(&state).unwrap();
    let _stop = StopDaemon(&iso, &state);
    let mut child = pump_cmd(&iso).spawn_owned().unwrap();
    let (peer, _stdout) = dial_through(&mut child);
    assert_eq!(peer.node, daemon_node(&state));
    wait_for_log(&state, "sync: link up ");
    assert!(state.join("node/node.key").exists());
}

/// A node never used locally (no state dir yet) still accepts an inbound link: the pump
/// creates the default state dir and starts the daemon there, as a first local use would.
#[test]
fn pump_on_a_fresh_node_creates_the_default_state_dir() {
    let iso = TestIsolation::new("peer-stdio-fresh-node");
    let state = iso.path("xdg/state").join("herdr/plugins/herdr-threads");
    assert!(!state.exists(), "the isolation starts with no state dir");
    let _stop = StopDaemon(&iso, &state);
    let mut child = pump_cmd(&iso).spawn_owned().unwrap();
    let (peer, _stdout) = dial_through(&mut child);
    assert_eq!(peer.node, daemon_node(&state));
    wait_for_log(&state, "sync: link up ");
}

/// sshd relays the pump's stdout from one thread with non-blocking reads. On Linux, a pump
/// that copied with `io::copy` spliced the socket into stdout, and a splice waiting on an
/// idle socket holds the stdout pipe's lock: sshd's read then blocked in the kernel, sshd
/// never forwarded the dialer's hello, and every real-ssh link timed out its handshake.
/// After the preamble, a non-blocking read of an idle pump's stdout must return at once.
#[test]
fn idle_pump_stdout_never_blocks_a_nonblocking_reader() {
    use std::os::fd::AsRawFd;
    let iso = TestIsolation::new("peer-stdio-nonblocking-relay");
    let state = iso.path("state");
    let _stop = StopDaemon(&iso, &state);
    let mut child = pump(&iso, &state);
    let mut stdout = child.stdout.take().unwrap();
    let _stdin = child.stdin.take().unwrap();
    let fd = stdout.as_raw_fd();
    // SAFETY: fd is the open read end of the child's stdout pipe, owned by `stdout`.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        assert!(flags >= 0);
        assert_eq!(libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK), 0);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let deadline = Instant::now() + WAIT;
        let mut got = Vec::new();
        let mut buf = [0u8; 64];
        while got.len() < PREAMBLE.len() {
            match stdout.read(&mut buf) {
                Ok(0) => panic!("pump closed stdout before the preamble"),
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "no preamble");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("reading the pump's stdout: {error}"),
            }
        }
        assert_eq!(&got[..], PREAMBLE);
        // Let the pump's daemon-to-stdout side settle on the idle socket.
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        let idle = stdout.read(&mut buf);
        let _ = tx.send((idle.map_err(|e| e.kind()), started.elapsed()));
    });
    let (idle, took) = rx.recv_timeout(Duration::from_secs(5)).expect(
        "a non-blocking read of the idle pump's stdout blocked (splice holds the pipe lock)",
    );
    assert_eq!(
        idle,
        Err(std::io::ErrorKind::WouldBlock),
        "idle pump has nothing to say"
    );
    assert!(
        took < Duration::from_secs(1),
        "non-blocking read took {took:?}"
    );
    drop(child);
}
