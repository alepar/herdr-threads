//! ht-p03.21: configuration smoke. Each Herdr up/down state runs once through
//! `daemon ensure`, `daemon health --json` and `doctor --json` against an
//! isolated named Herdr session (never the shared server). The tests assert
//! only that every command completes with a recorded exit code; the outcome
//! classification lives in
//! `docs/history/remaining-findings-run/config-smoke.md`.
//! Set `HT_CONFIG_SMOKE_OUT` to capture `<state>/<step>.{out,err,code}`.
use herdr_threads::test_support::isolated_herdr::{HerdrState, IsolatedHerdr};
use std::{
    fs,
    path::{Path, PathBuf},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Smoke {
    herdr: IsolatedHerdr,
    state: &'static str,
    plugin_state: PathBuf,
}

impl Smoke {
    fn new(state: &'static str) -> Option<Self> {
        let herdr = IsolatedHerdr::new(state)?;
        let plugin_state = herdr.root().join("plugin-state");
        fs::create_dir_all(&plugin_state).unwrap();
        Some(Self {
            herdr,
            state,
            plugin_state,
        })
    }

    fn cli(&self, args: &[&str]) -> (i32, String, String) {
        let output = self
            .herdr
            .command(BIN)
            .arg("--json")
            .arg("--state-dir")
            .arg(&self.plugin_state)
            .arg("--host-endpoint")
            .arg(self.herdr.socket_path())
            .args(args)
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("HERDR_ENV")
            .output()
            .unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// Run one step, print and (when asked) record its output, return the exit code.
    fn step(&self, name: &str, args: &[&str]) -> i32 {
        let (code, out, err) = self.cli(args);
        println!(
            "[{}] {name}: exit={code}\n  stdout: {}\n  stderr: {}",
            self.state,
            out.trim(),
            err.trim()
        );
        if let Some(dir) = std::env::var_os("HT_CONFIG_SMOKE_OUT") {
            let dir = Path::new(&dir).join(self.state);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(format!("{name}.out")), &out).unwrap();
            fs::write(dir.join(format!("{name}.err")), &err).unwrap();
            fs::write(dir.join(format!("{name}.code")), format!("{code}\n")).unwrap();
        }
        assert_ne!(code, -1, "{name} was killed by a signal");
        code
    }

    fn triple(&self, label: &str) {
        self.step(&format!("{label}-ensure"), &["daemon", "ensure"]);
        self.step(&format!("{label}-health"), &["daemon", "health"]);
        self.step(&format!("{label}-doctor"), &["doctor"]);
    }
}

impl Drop for Smoke {
    fn drop(&mut self) {
        // Every daemon a smoke test starts is stopped before the test ends.
        let _ = self.cli(&["daemon", "stop"]);
    }
}

#[test]
fn herdr_up_linked() {
    let Some(s) = Smoke::new("herdr_up_linked") else {
        return;
    };
    s.herdr.start();
    assert_eq!(s.herdr.state(), HerdrState::Up);
    s.triple("up");
}

#[test]
fn herdr_stopped_while_daemon_runs() {
    let Some(s) = Smoke::new("herdr_stopped_while_daemon_runs") else {
        return;
    };
    s.herdr.start();
    s.step("before-ensure", &["daemon", "ensure"]);
    s.herdr.stop();
    assert_eq!(s.herdr.state(), HerdrState::Stopped);
    s.step("stopped-health", &["daemon", "health"]);
    s.step("stopped-doctor", &["doctor"]);
}

#[test]
fn herdr_never_started() {
    let Some(s) = Smoke::new("herdr_never_started") else {
        return;
    };
    assert_eq!(s.herdr.state(), HerdrState::NeverStarted);
    assert!(!s.herdr.socket_path().exists());
    s.triple("never");
}

#[test]
fn herdr_stale_socket_dead_server() {
    let Some(s) = Smoke::new("herdr_stale_socket_dead_server") else {
        return;
    };
    s.herdr.stale_socket();
    assert_eq!(s.herdr.state(), HerdrState::StaleSocket);
    s.triple("stale");
}

#[test]
fn herdr_restarted_under_running_daemon() {
    let Some(s) = Smoke::new("herdr_restarted_under_running_daemon") else {
        return;
    };
    s.herdr.start();
    s.step("before-ensure", &["daemon", "ensure"]);
    s.herdr.restart();
    assert_eq!(s.herdr.state(), HerdrState::Up);
    assert_eq!(s.herdr.starts(), 2);
    s.step("restarted-health", &["daemon", "health"]);
    s.step("restarted-doctor", &["doctor"]);
}
