//! The fixture of the named-seats end-to-end suite (ht-kqrt.34): one real `herdr-threads daemon
//! run` child over a scratch state directory, and the real binary run as an agent stand-in. A
//! standalone rig has no Herdr and a `PATH` that holds no `herdr`; a Herdr rig adds one isolated,
//! named Herdr server whose socket the same daemon is given. Every child is a `test_support`
//! owned child; `Drop` stops the daemon (and, for a Herdr rig, the server, through
//! `IsolatedHerdr`).

use herdr_threads::test_support::{
    isolated_herdr::IsolatedHerdr,
    isolation::{TestIsolation, as_agent},
    resolve_seeded,
    spawn::{OwnedChild, SpawnOwned},
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

pub struct Ran {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl std::fmt::Debug for Ran {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "exit {:?}\nstdout: {}\nstderr: {}",
            self.code, self.stdout, self.stderr
        )
    }
}

pub struct Rig {
    daemon: Option<OwnedChild>,
    pub isolation: TestIsolation,
    pub herdr: Option<IsolatedHerdr>,
    pub state: PathBuf,
    /// The directory every standalone child has as its whole `PATH`; a Herdr rig puts it first.
    pub bin: PathBuf,
}

pub fn run(mut command: Command, stdin: Option<&[u8]>) -> Ran {
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
    Ran {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Polls `probe` until it returns `Ok`; the bound is a hang guard, never a wait.
pub fn wait_for<T>(
    what: &str,
    timeout: Duration,
    mut probe: impl FnMut() -> Result<T, String>,
) -> T {
    let until = Instant::now() + timeout;
    loop {
        match probe() {
            Ok(value) => return value,
            Err(last) if Instant::now() >= until => panic!("{what}: timed out: {last}"),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

pub fn executable(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

impl Rig {
    /// No Herdr anywhere: the daemon has no host endpoint and `PATH` holds no `herdr`.
    pub fn standalone(label: &str) -> Self {
        Self::standalone_with(label, &[])
    }

    /// [`Self::standalone`] with extra environment for the daemon (a test's idle-exit period).
    pub fn standalone_with(label: &str, daemon_env: &[(&str, &str)]) -> Self {
        let isolation = TestIsolation::new(label);
        let state = std::fs::canonicalize(isolation.state_root())
            .expect("the state root exists")
            .join("state");
        std::fs::create_dir_all(&state).unwrap();
        resolve_seeded(&state);
        let bin = isolation.path("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mut rig = Self {
            daemon: None,
            isolation,
            herdr: None,
            state,
            bin,
        };
        rig.start_daemon(None, daemon_env);
        rig
    }

    /// An isolated Herdr server (a `codex` stand-in first on its `PATH`, for a handoff) and one
    /// daemon given its socket. `None` when Herdr is absent and `HT_SKIP_HERDR_TESTS=1`.
    pub fn beside_herdr(case: &str) -> Option<Self> {
        let herdr = IsolatedHerdr::new(case)?;
        let isolation = TestIsolation::new(case);
        let state = std::fs::canonicalize(isolation.state_root())
            .expect("the state root exists")
            .join("state");
        std::fs::create_dir_all(&state).unwrap();
        resolve_seeded(&state);
        let bin = isolation.path("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(herdr.root().join("cfg")).unwrap();
        std::fs::write(
            herdr.root().join("cfg/herdr.toml"),
            "default_shell = '/bin/sh'\n",
        )
        .unwrap();
        std::fs::create_dir_all(herdr.root().join("home")).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/herdr-matrix");
        executable(
            &bin.join("codex"),
            &format!(
                "#!/bin/sh\nexport FAKE_CODEX_LOG_DIR=\"$HOME/.fake-codex\"\nexec python3 -I {} \"$@\"\n",
                fixture.join("codex.js").display()
            ),
        );
        herdr.start_with_path_prefix(&bin);
        let socket = herdr.socket_path();
        let mut rig = Self {
            daemon: None,
            isolation,
            herdr: Some(herdr),
            state,
            bin,
        };
        rig.start_daemon(Some(&socket), &[]);
        Some(rig)
    }

    fn start_daemon(&mut self, endpoint: Option<&Path>, env: &[(&str, &str)]) {
        let mut command = self.base(BIN);
        command
            .args(["daemon", "run", "--state-dir"])
            .arg(&self.state);
        if let Some(endpoint) = endpoint {
            command.arg("--host-endpoint").arg(endpoint);
        }
        command
            .env(
                "HT_TEST_LANE_TICK_MS",
                herdr_threads::test_support::TEST_DAEMON_LANE_TICK_MS,
            )
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(self.isolation.path("daemon.stderr")).unwrap());
        self.daemon = Some(command.spawn_owned().expect("spawn the daemon"));
        self.wait_for_daemon();
    }

    /// Stops the daemon through the CLI and starts a fresh one over the same state directory
    /// (a standalone rig: no host endpoint). Everything the daemon held in memory is lost.
    pub fn restart_daemon(&mut self) {
        let stopped = self.plain(&["daemon", "stop"]);
        assert_eq!(stopped.code, Some(0), "daemon stop: {stopped:?}");
        self.daemon.take();
        self.start_daemon(None, &[]);
    }

    pub fn daemon_exited(&mut self) -> bool {
        self.daemon
            .as_mut()
            .is_some_and(|daemon| matches!(daemon.try_wait(), Ok(Some(_))))
    }

    fn wait_for_daemon(&self) {
        let until = Instant::now() + Duration::from_secs(30);
        while Instant::now() < until {
            if self.plain(&["daemon", "health"]).code == Some(0) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!(
            "the daemon did not come up: {}",
            std::fs::read_to_string(self.isolation.path("daemon.stderr")).unwrap_or_default()
        );
    }

    /// `program` with the rig's scrubbed, isolated environment. A standalone rig's `PATH` is the
    /// empty `bin` directory; a Herdr rig keeps the real `PATH` behind it.
    pub fn base(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = match &self.herdr {
            Some(herdr) => {
                let mut command = herdr.command(program);
                let path = std::env::var("PATH").unwrap_or_default();
                command
                    .env("PATH", format!("{}:{path}", self.bin.display()))
                    .env("CODEX_HOME", herdr.root().join("home/.codex"))
                    .env("CLAUDE_CONFIG_DIR", herdr.root().join("home/.claude"));
                command
            }
            None => {
                let mut command = self.isolation.command(program);
                command.env("PATH", &self.bin);
                command
            }
        };
        command.env("NO_COLOR", "1");
        command
    }

    pub fn command(&self) -> Command {
        let mut command = self.base(BIN);
        command.arg("--state-dir").arg(&self.state);
        command
    }

    pub fn plain(&self, args: &[&str]) -> Ran {
        let mut command = self.command();
        command.args(args);
        run(command, None)
    }

    /// `args` as the agent of `session` named `name`: no pane, no `--cooperative-*`.
    pub fn agent(&self, name: &str, session: &str, args: &[&str]) -> Ran {
        self.agent_with(name, session, args, None)
    }

    pub fn agent_with(
        &self,
        name: &str,
        session: &str,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> Ran {
        let mut command = self.command();
        command.args(args).env("HERDR_THREADS_SEAT_NAME", name);
        as_agent(&mut command, "claude", session);
        run(command, stdin)
    }

    /// `args` as the agent of `session` in Herdr pane `pane`.
    pub fn pane_agent(&self, pane: &str, session: &str, args: &[&str]) -> Ran {
        self.pane_agent_with(pane, session, args, None)
    }

    pub fn pane_agent_with(
        &self,
        pane: &str,
        session: &str,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> Ran {
        let mut command = self.command();
        command
            .args(args)
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", pane);
        as_agent(&mut command, "claude", session);
        run(command, stdin)
    }

    fn hook_payload(session: &str, event: &str, extra: &str) -> String {
        format!(
            r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"{event}"{extra}}}"#
        )
    }

    /// The SessionStart hook of `session` under `name`.
    pub fn session_start(&self, name: &str, session: &str) -> Ran {
        let payload = Self::hook_payload(session, "SessionStart", r#","source":"startup""#);
        self.agent_with(name, session, &["hook", "claude"], Some(payload.as_bytes()))
    }

    /// The UserPromptSubmit hook of `session` under `name`: the turn-start delivery.
    pub fn named_prompt_hook(&self, name: &str, session: &str) -> Ran {
        let payload = Self::hook_payload(session, "UserPromptSubmit", r#","prompt":"go""#);
        self.agent_with(name, session, &["hook", "claude"], Some(payload.as_bytes()))
    }

    /// The SessionStart hook of `session` in `pane`.
    pub fn pane_session_start(&self, pane: &str, session: &str) -> Ran {
        let payload = Self::hook_payload(session, "SessionStart", r#","source":"startup""#);
        self.pane_agent_with(pane, session, &["hook", "claude"], Some(payload.as_bytes()))
    }

    /// The UserPromptSubmit hook of `session` in `pane`.
    pub fn pane_prompt_hook(&self, pane: &str, session: &str) -> Ran {
        let payload = Self::hook_payload(session, "UserPromptSubmit", r#","prompt":"go""#);
        self.pane_agent_with(pane, session, &["hook", "claude"], Some(payload.as_bytes()))
    }

    /// A turn-start hook until its `additionalContext` holds `want` (delivery follows the
    /// daemon's attention cadence); returns the context.
    pub fn hook_context(&self, want: &str, hook: impl Fn() -> Ran) -> String {
        wait_for(
            &format!("a hook delivering {want:?}"),
            Duration::from_secs(30),
            || {
                let ran = hook();
                let value: serde_json::Value =
                    serde_json::from_str(&ran.stdout).map_err(|e| format!("{e}: {ran:?}"))?;
                let context = value["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if context.contains(want) {
                    Ok(context)
                } else {
                    Err(format!("{ran:?}"))
                }
            },
        )
    }

    /// The SessionStart hook until it enrolls: the named host's first observation of a new seat
    /// lands on the daemon's observation cadence, so the first check-ins answer "unavailable".
    pub fn enroll(&self, name: &str, session: &str) {
        wait_for(&format!("{name} enrolled"), Duration::from_secs(60), || {
            let hook = self.session_start(name, session);
            if hook.stdout.contains("Your seat") {
                Ok(())
            } else {
                Err(format!("{hook:?}"))
            }
        });
    }

    /// The same for a seat in `pane`.
    pub fn enroll_pane(&self, pane: &str, session: &str) {
        wait_for(&format!("{pane} enrolled"), Duration::from_secs(60), || {
            let hook = self.pane_session_start(pane, session);
            if hook.stdout.contains("Your seat") {
                Ok(())
            } else {
                Err(format!("{hook:?}"))
            }
        });
    }

    /// A fresh `herdr workspace create` root pane (never a pane the test did not make).
    pub fn pane(&self, label: &str) -> String {
        let herdr = self.herdr.as_ref().expect("a Herdr rig");
        let out = herdr
            .command("herdr")
            .args([
                "workspace",
                "create",
                "--label",
                label,
                "--no-focus",
                "--cwd",
            ])
            .arg(herdr.root().join("home"))
            .env("PATH", {
                let path = std::env::var("PATH").unwrap_or_default();
                format!("{}:{path}", self.bin.display())
            })
            .output()
            .unwrap();
        assert!(out.status.success(), "workspace create: {out:?}");
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        value["result"]["root_pane"]["pane_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    pub fn db(&self) -> rusqlite::Connection {
        let paths = resolve_seeded(&self.state);
        let db = rusqlite::Connection::open(paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }

    pub fn seat_count(&self) -> i64 {
        self.db()
            .query_row("SELECT COUNT(*) FROM seats", [], |r| r.get(0))
            .unwrap()
    }

    pub fn seat_of(&self, name: &str) -> String {
        self.db()
            .query_row(
                "SELECT id FROM seats WHERE name=?1 AND state<>'retired'",
                [name],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("a seat named {name}: {e}"))
    }

    pub fn seat_of_pane(&self, pane: &str) -> String {
        self.db()
            .query_row(
                "SELECT id FROM seats WHERE target_id=?1 AND state<>'retired'",
                [pane],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("a seat on pane {pane}: {e}"))
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // The daemon stops before the Herdr server under it (`IsolatedHerdr` drops after).
        let _ = self.plain(&["daemon", "stop"]);
        self.daemon.take();
    }
}

pub fn sent_message(ran: &Ran) -> String {
    let words: Vec<&str> = ran.stdout.split_whitespace().collect();
    let at = words
        .iter()
        .position(|w| *w == "message")
        .unwrap_or_else(|| panic!("a message id in {ran:?}"));
    words[at + 1].to_owned()
}
