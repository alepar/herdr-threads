//! Isolated named Herdr test sessions (ht-p03.1): drives
//! `scripts/lib/isolated-herdr.sh`; one private server per fixture,
//! torn down on drop (also while unwinding from a panic). After an abnormal
//! exit (SIGTERM or SIGKILL) the per-process reaper (`spawn::ensure_reaper`)
//! tears down every root this test process owned.
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub const SCRIPT: &str = crate::test_support::spawn::LIB;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Present(PathBuf),
    Skip,
    Missing,
}

/// The test Herdr-availability policy, pure so it is testable without
/// touching the process environment. Present when some PATH entry holds an
/// executable `herdr`; else Skip when `skip == Some("1")`; else Missing.
pub fn availability(path: Option<&OsStr>, skip: Option<&OsStr>) -> Availability {
    use std::os::unix::fs::PermissionsExt;
    if let Some(path) = path {
        for dir in std::env::split_paths(path) {
            let candidate = dir.join("herdr");
            let executable = std::fs::metadata(&candidate)
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false);
            if executable {
                return Availability::Present(candidate);
            }
        }
    }
    if skip == Some(OsStr::new("1")) {
        Availability::Skip
    } else {
        Availability::Missing
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HerdrState {
    Up,
    Stopped,
    NeverStarted,
    StaleSocket,
}

pub struct IsolatedHerdr {
    root: PathBuf,
    case: String,
}

fn helper(args: &[&OsStr]) -> Output {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(r#". "$1"; shift; "$@""#)
        .arg("sh")
        .arg(SCRIPT)
        .args(args)
        // Roots are owned by this test process and the reaper can be started.
        .env("IH_LIB", SCRIPT);
    crate::test_support::spawn::tag(&mut cmd);
    cmd.output().expect("spawn sh for isolated-herdr helper")
}

impl IsolatedHerdr {
    /// A fresh private root (no server yet). `None` (after printing
    /// "skipped: no herdr: <case>") when Herdr is absent and
    /// HT_SKIP_HERDR_TESTS=1; panics naming the missing `herdr` binary otherwise.
    pub fn new(case: &str) -> Option<Self> {
        let path = std::env::var_os("PATH");
        let skip = std::env::var_os("HT_SKIP_HERDR_TESTS");
        match availability(path.as_deref(), skip.as_deref()) {
            Availability::Present(_) => {}
            Availability::Skip => {
                println!("skipped: no herdr: {case}");
                return None;
            }
            Availability::Missing => panic!(
                "isolated-herdr: required binary 'herdr' not found on PATH (case: {case}; set HT_SKIP_HERDR_TESTS=1 to skip)"
            ),
        }
        let out = helper(&[OsStr::new("ih_root_new")]);
        assert!(
            out.status.success(),
            "ih_root_new failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        Some(Self {
            root,
            case: case.to_owned(),
        })
    }

    fn run(&self, verb: &str, with_case: bool) -> Output {
        let mut args: Vec<&OsStr> = vec![OsStr::new(verb), self.root.as_os_str()];
        if with_case {
            args.push(OsStr::new(&self.case));
        }
        helper(&args)
    }

    fn run_ok(&self, verb: &str, with_case: bool) {
        let out = self.run(verb, with_case);
        assert!(
            out.status.success(),
            "{verb} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    pub fn start(&self) {
        self.run_ok("ih_start", true);
    }
    pub fn stop(&self) {
        self.run_ok("ih_stop", false);
    }
    pub fn kill(&self) {
        self.run_ok("ih_kill", false);
    }
    pub fn stale_socket(&self) {
        self.run_ok("ih_stale_socket", true);
    }
    pub fn restart(&self) {
        self.run_ok("ih_restart", true);
    }

    pub fn state(&self) -> HerdrState {
        let out = self.run("ih_state", false);
        match String::from_utf8_lossy(&out.stdout).trim() {
            "up" => HerdrState::Up,
            "stopped" => HerdrState::Stopped,
            "never-started" => HerdrState::NeverStarted,
            "stale-socket" => HerdrState::StaleSocket,
            other => panic!("unexpected ih_state output {other:?}"),
        }
    }

    /// How many times the server has been started under this root.
    pub fn starts(&self) -> u32 {
        std::fs::read_to_string(self.root.join("starts"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    pub fn pid(&self) -> Option<u32> {
        std::fs::read_to_string(self.root.join("server.pid"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn socket_path(&self) -> PathBuf {
        self.root.join("h.sock")
    }

    /// The private environment a spawned binary needs (HOME, XDG_*,
    /// HERDR_CONFIG_PATH, HERDR_SOCKET_PATH).
    pub fn env(&self) -> Vec<(OsString, OsString)> {
        let r = &self.root;
        [
            ("HOME", r.join("home")),
            ("XDG_CONFIG_HOME", r.join("cfg")),
            ("XDG_STATE_HOME", r.join("st")),
            ("XDG_RUNTIME_DIR", r.join("rt")),
            ("HERDR_CONFIG_PATH", r.join("cfg/herdr.toml")),
            ("HERDR_SOCKET_PATH", r.join("h.sock")),
        ]
        .into_iter()
        .map(|(k, v)| (OsString::from(k), v.into_os_string()))
        .collect()
    }

    /// `program` with every inherited HERDR_* removed and `env()` applied.
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut cmd = Command::new(program);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HERDR_") {
                cmd.env_remove(key);
            }
        }
        cmd.envs(self.env());
        // Owner tag and owner pid: a daemon started through this command
        // exits with the test process (ht-p03.131).
        crate::test_support::spawn::tag(&mut cmd);
        cmd
    }
}

impl Drop for IsolatedHerdr {
    fn drop(&mut self) {
        // Flake diagnosis: a failing test with HT_KEEP_FAILED_ROOTS=1 keeps
        // its root (daemon log, database, server output); processes still stop.
        let keep = std::thread::panicking()
            && std::env::var_os("HT_KEEP_FAILED_ROOTS").is_some_and(|v| v == "1");
        let out = self.run(if keep { "ih_keep" } else { "ih_teardown" }, false);
        if keep {
            eprintln!(
                "isolated-herdr: kept failed test root {}",
                self.root.display()
            );
        }
        if !out.status.success() {
            eprintln!(
                "isolated-herdr: teardown of {} failed: {}",
                self.root.display(),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
