//! ht-p03.10: Herdr never started and a stale Herdr socket both read as
//! "server not running" through the daemon's health and `doctor`, against an
//! isolated named Herdr test session (never the shared server).
use herdr_threads::{
    daemon::remedy::{RemedyContext, remedy},
    protocol::results::ErrorClass,
    test_support::isolated_herdr::{HerdrState, IsolatedHerdr},
};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Scratch {
    root: PathBuf,
    state: PathBuf,
    host: PathBuf,
}
impl Scratch {
    fn new(host: &Path) -> Self {
        let root = PathBuf::from(format!(
            "/private/tmp/hths-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let state = root.join("st");
        fs::DirBuilder::new().mode(0o700).create(&state).unwrap();
        Self {
            root,
            state,
            host: host.to_path_buf(),
        }
    }
    fn cli(&self, args: &[&str]) -> Output {
        crate::scrubbed_command(BIN)
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.host)
            .args(args)
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"))
            .env("CODEX_HOME", self.root.join("codex-home"))
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .output()
            .unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.cli(&["daemon", "stop"]);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The daemon's own account of the host, through `ensure`, `health` and
/// `doctor`: the host is unavailable and the reason says the server is not
/// running (not a witness or decode detail).
fn assert_reports_server_not_running(host: &Path) {
    let scratch = Scratch::new(host);
    let ensured = scratch.cli(&["daemon", "ensure"]);
    assert!(
        ensured.status.success(),
        "a degraded daemon still ensures: {}",
        text(&ensured.stderr)
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let health = loop {
        let health = text(&scratch.cli(&["daemon", "health"]).stdout);
        if health.contains("server not running") || Instant::now() > deadline {
            break health;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(health.contains("host unavailable"), "{health}");
    assert!(health.contains("server not running"), "{health}");
    let doctor = text(&scratch.cli(&["doctor"]).stdout);
    assert!(
        doctor.contains("daemon.host_reachability: unavailable"),
        "{doctor}"
    );
    assert!(doctor.contains("server not running"), "{doctor}");
}

/// Kills: the host connect path reporting a missing endpoint as a witness /
/// stale-observation detail instead of "server not running".
#[test]
fn never_started_reports_server_not_running() {
    let Some(herdr) = IsolatedHerdr::new("never_started_reports_server_not_running") else {
        return;
    };
    assert_eq!(herdr.state(), HerdrState::NeverStarted);
    assert!(!herdr.socket_path().exists());
    assert_reports_server_not_running(&herdr.socket_path());
}

/// Kills: a refused connect to a dead server's leftover socket being reported
/// as a transport error instead of "server not running".
#[test]
fn stale_socket_reports_server_not_running() {
    let Some(herdr) = IsolatedHerdr::new("stale_socket_reports_server_not_running") else {
        return;
    };
    herdr.stale_socket();
    assert_eq!(herdr.state(), HerdrState::StaleSocket);
    assert!(herdr.socket_path().exists());
    assert_reports_server_not_running(&herdr.socket_path());
}

/// Kills: Health staying silent about where to look when Herdr stops under a
/// running daemon (a lane failed since its last success, so Health must say
/// the `remedy()` degraded pointer naming daemon.log), and doctor omitting the log path it points
/// at. Isolated named Herdr session; the shared server is never touched.
#[test]
fn herdr_stopped_health_says_degraded_see_log() {
    let Some(herdr) = IsolatedHerdr::new("herdr_stopped_health_says_degraded_see_log") else {
        return;
    };
    herdr.start();
    assert_eq!(herdr.state(), HerdrState::Up);
    let scratch = Scratch::new(&herdr.socket_path());
    let ensured = scratch.cli(&["daemon", "ensure"]);
    assert!(ensured.status.success(), "{}", text(&ensured.stderr));
    // Wait for the daemon to verify the running host once.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let doctor = text(&scratch.cli(&["doctor"]).stdout);
        if doctor.contains("daemon.host_reachability: ready") || Instant::now() > deadline {
            assert!(
                doctor.contains("daemon.host_reachability: ready"),
                "host never became reachable: {doctor}"
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    herdr.stop();
    assert_eq!(herdr.state(), HerdrState::Stopped);
    let doctor_log = text(&scratch.cli(&["doctor"]).stdout)
        .lines()
        .find_map(|line| line.strip_prefix("daemon_log: ").map(str::to_owned))
        .expect("doctor prints the daemon log path");
    assert!(doctor_log.ends_with("daemon.log"), "{doctor_log}");
    let pointers = lane_pointers(&doctor_log);
    let shows_pointer = |output: &str| pointers.iter().any(|pointer| output.contains(pointer));
    let deadline = Instant::now() + Duration::from_secs(40);
    let health = loop {
        let health = text(&scratch.cli(&["daemon", "health"]).stdout);
        if shows_pointer(&health) || Instant::now() > deadline {
            break health;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(shows_pointer(&health), "{pointers:?} not in {health}");
    let doctor = text(&scratch.cli(&["doctor"]).stdout);
    assert!(shows_pointer(&doctor), "{pointers:?} not in {doctor}");
    assert!(
        doctor.contains(&format!("daemon_log: {doctor_log}\n")),
        "{doctor}"
    );
}

/// Every `degraded: <remedy>` pointer the failing lane's class could yield (the
/// class is not fixed in advance).
fn lane_pointers(log: &str) -> Vec<String> {
    [
        None,
        Some(ErrorClass::Transient),
        Some(ErrorClass::Unavailable),
        Some(ErrorClass::Corrupt),
    ]
    .into_iter()
    .map(|class| {
        format!(
            "degraded: {}",
            remedy(class, &RemedyContext::LaneDegraded { log: log.into() })
        )
    })
    .collect()
}
