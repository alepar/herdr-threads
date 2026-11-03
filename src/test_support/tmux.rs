//! A private tmux server for tests (N2, spike T4). `PrivateTmux::start` runs
//! `tmux -L ht-test-<uuid> -f /dev/null new-session -d` through `test_support::spawn`, so the
//! server carries the owner tag the reaper and `scripts/check-no-leaked-processes` read, and Drop
//! runs `kill-server` on the private socket. Never the user's own tmux: the socket name is the
//! fixture's, and `-f /dev/null` loads no config.
use std::{
    path::PathBuf,
    process::{Command, Output},
    time::{Duration, Instant},
};

use crate::test_support::spawn::{self, SpawnOwned};

pub struct PrivateTmux {
    name: String,
    /// The server socket, read at start: `kill-server` leaves the file.
    socket: PathBuf,
}

/// One pane of the private server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    /// `%<n>`, the value of `TMUX_PANE` in that pane.
    pub id: String,
    pub pid: u32,
}

/// A missing `tmux`: the skip line, or a panic naming `case` when tmux is required.
fn on_missing(required: bool, case: &str) {
    if required {
        panic!("tmux required (HT_REQUIRE_TMUX set) but not found on PATH (case: {case})");
    }
    println!("skipped: no tmux: {case}");
}

impl PrivateTmux {
    /// A fresh server with one window running `sleep 600`. `None` (after printing
    /// `skipped: no tmux: <case>`, the recorded reason) when `tmux` is not installed; with
    /// `HT_REQUIRE_TMUX` set (CI), a missing `tmux` panics instead.
    pub fn start(case: &str) -> Option<Self> {
        let name = format!("ht-test-{}", uuid::Uuid::new_v4().simple());
        let mut command = spawn::command("tmux");
        command
            .args([
                "-L",
                &name,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "sleep 600",
            ])
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        let mut client = match command.spawn_owned() {
            Ok(client) => client,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                on_missing(std::env::var_os("HT_REQUIRE_TMUX").is_some(), case);
                return None;
            }
            Err(error) => panic!("tmux start failed: {error}"),
        };
        // The client exits once the server it forked is up; stop() after wait leaves the server.
        let status = client.wait().expect("wait for the tmux client");
        assert!(status.success(), "tmux new-session failed: {status}");
        let mut tmux = Self {
            name,
            socket: PathBuf::new(),
        };
        // `kill-server` leaves the socket file: removed on drop, and ledgered so a killed test's
        // socket is removed at exit or reported by `check-no-leaked-temp-files`.
        tmux.socket = tmux.socket_path();
        crate::test_support::scratch::register(&tmux.socket);
        Some(tmux)
    }

    /// `tmux -L <name> <args>`, its output.
    pub fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new("tmux");
        command
            .args(["-L", &self.name])
            .args(args)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        command.output().expect("run tmux")
    }

    fn text(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "tmux {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    pub fn socket_path(&self) -> PathBuf {
        PathBuf::from(self.text(&["display", "-p", "#{socket_path}"]))
    }

    pub fn server_pid(&self) -> u32 {
        self.text(&["display", "-p", "#{pid}"])
            .parse()
            .expect("tmux server pid")
    }

    /// A new detached window running `command` (through `sh -c`); its pane.
    pub fn new_pane(&self, command: &str) -> Pane {
        let text = self.text(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id} #{pane_pid}",
            command,
        ]);
        let (id, pid) = text.split_once(' ').expect("pane id and pid");
        Pane {
            id: id.to_owned(),
            pid: pid.parse().expect("pane pid"),
        }
    }

    /// The hint a caller in `pane` would send: `TMUX` and `TMUX_PANE` as tmux sets them.
    pub fn hint(&self, pane: &Pane) -> String {
        format!(
            "tmux:{}:{}:{}",
            self.socket_path().display(),
            self.server_pid(),
            pane.id
        )
    }

    /// Wait until the private server is gone.
    pub fn wait_gone(&self, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if !self.run(&["display", "-p", "x"]).status.success() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

impl Drop for PrivateTmux {
    fn drop(&mut self) {
        let _ = self.run(&["kill-server"]);
        if !self.socket.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.socket);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::on_missing;

    #[test]
    #[should_panic(expected = "case: wake-case")]
    fn missing_tmux_is_a_failure_when_required() {
        on_missing(true, "wake-case");
    }

    #[test]
    fn missing_tmux_is_a_skip_when_not_required() {
        on_missing(false, "wake-case");
    }
}
