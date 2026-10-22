//! ht-54r.19: configuration smoke for session-keyed seat bindings (design §4, configurations 1-9).
//!
//! Each test builds one configuration against its own isolated named Herdr session (never the
//! shared server), a real detached daemon and isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME /
//! HERMES_HOME, and drives the real `herdr-threads` executable (`hook <harness>` and the CLI)
//! with synthetic payloads and environment. No model and no real harness binary runs. A caller is
//! either a long-lived stand-in harness process (`isolation::as_agent`, a stable fingerprint) or a
//! synthetic harness parent (`harness_shim`, real ancestry, a fingerprint that dies with the call).
//! Assertions read the daemon's database and the executable's output.
use herdr_threads::test_support::{
    harness_shim,
    isolated_herdr::IsolatedHerdr,
    isolation::{as_agent, scrub_env},
    spawn::{self, SpawnOwned},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

pub(super) const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
/// Exit status of a refused agent call (`EXIT_UNSUPPORTED`).
pub(super) const REFUSED: i32 = 4;

/// Polls `done` until it holds; the bound is a hang guard, never a wait.
pub(super) fn wait_until(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// One agent process of a configuration.
#[derive(Clone)]
pub(super) struct Agent {
    pub(super) harness: &'static str,
    pub(super) session: String,
    pub(super) pane: String,
    /// `Some(thread)`: Codex's `CODEX_THREAD_ID` (a subagent when it differs from the session).
    pub(super) thread: Option<String>,
    /// Real ancestry through a synthetic harness parent instead of a stand-in process.
    pub(super) shim: bool,
    /// Export no session variable (a pre-upgrade agent).
    pub(super) no_session: bool,
    /// The shim's parent is `codex app-server --managed-daemon ...`.
    pub(super) managed_daemon: bool,
    /// The pid of an existing stand-in process this agent runs inside (a `/clear` or a resume is
    /// the same harness process under a new session id).
    pub(super) process: Option<i64>,
    /// Variables inherited from an outer harness (the `hermes -z` from Claude's Bash tool case).
    pub(super) inherited: Vec<(&'static str, String)>,
}
impl Agent {
    pub(super) fn new(harness: &'static str, session: &str, pane: &str) -> Self {
        Self {
            harness,
            session: session.to_owned(),
            pane: pane.to_owned(),
            thread: None,
            shim: false,
            managed_daemon: false,
            no_session: false,
            process: None,
            inherited: Vec::new(),
        }
    }
    pub(super) fn claude(session: &str, pane: &str) -> Self {
        Self::new("claude", session, pane)
    }
    pub(super) fn codex(session: &str, pane: &str) -> Self {
        Self::new("codex", session, pane)
    }
    pub(super) fn hermes(session: &str, pane: &str) -> Self {
        Self::new("hermes", session, pane)
    }
    pub(super) fn shimmed(mut self) -> Self {
        self.shim = true;
        self
    }
    pub(super) fn under_managed_daemon(mut self) -> Self {
        self.shim = true;
        self.managed_daemon = true;
        self
    }
    pub(super) fn in_process_of(mut self, binding: &Binding) -> Self {
        self.process = Some(binding.pid.expect("the binding carries its harness pid"));
        self
    }
    pub(super) fn in_process(mut self, pid: i64) -> Self {
        self.process = Some(pid);
        self
    }
    pub(super) fn thread(mut self, thread: &str) -> Self {
        self.thread = Some(thread.to_owned());
        self
    }
    pub(super) fn inheriting(mut self, variable: &'static str, value: &str) -> Self {
        self.inherited.push((variable, value.to_owned()));
        self
    }
    pub(super) fn session_variable(&self) -> &'static str {
        match self.harness {
            "claude" => "CLAUDE_CODE_SESSION_ID",
            "codex" => "CODEX_SESSION_ID",
            _ => "HERMES_SESSION_ID",
        }
    }
}

/// A finished command.
pub(super) struct Out {
    pub(super) code: i32,
    pub(super) stdout: String,
    pub(super) stderr: String,
}
impl Out {
    pub(super) fn of(output: Output) -> Self {
        Self {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
    pub(super) fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}{}", self.stdout, self.stderr))
    }
    /// `result.data` of a successful `--json` call.
    pub(super) fn data(&self) -> Value {
        assert_eq!(self.code, 0, "{}{}", self.stdout, self.stderr);
        self.json()["result"]["data"].clone()
    }
    pub(super) fn text(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// An open or ended binding row.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Binding {
    pub(super) id: String,
    pub(super) harness: String,
    pub(super) session: String,
    pub(super) role: String,
    pub(super) agent_id: String,
    pub(super) liveness: String,
    pub(super) reason: Option<String>,
    pub(super) provenance: String,
    pub(super) pid: Option<i64>,
    pub(super) ended: bool,
    pub(super) parent: Option<String>,
}

pub(super) struct Smoke {
    pub(super) herdr: IsolatedHerdr,
    pub(super) state: PathBuf,
    pub(super) cwd: PathBuf,
    pub(super) labels: usize,
    /// Stand-in harness processes started by `stand_in`, stopped with the fixture.
    pub(super) processes: Vec<spawn::OwnedChild>,
    /// `Some`: the daemon's lanes (liveness re-check, observation cadence, wake and deadline
    /// safety ticks) run on this test-support tick instead of 5 s.
    pub(super) lane_tick: Option<Duration>,
}

/// The test-support lane tick of [`Smoke::ticking`]: the configurations whose waits are set by
/// the 5 s liveness and observation cadences, not by what they prove, run on it.
pub(super) const SMOKE_TICK: Duration = Duration::from_secs(1);

impl Smoke {
    /// An isolated Herdr, state dir and daemon. `None` (after `IsolatedHerdr` prints
    /// `skipped: no herdr: <case>`) when herdr is absent and `HT_SKIP_HERDR_TESTS=1`.
    pub(super) fn new(case: &'static str) -> Option<Self> {
        Self::with_settings(case, "")
    }

    /// [`Self::new`] with the daemon's lanes on [`SMOKE_TICK`].
    pub(super) fn ticking(case: &'static str) -> Option<Self> {
        Self::build(case, "", Some(SMOKE_TICK))
    }

    /// `extra` is spliced into the daemon's settings object (`"key":value,`).
    pub(super) fn with_settings(case: &'static str, extra: &str) -> Option<Self> {
        Self::build(case, extra, None)
    }

    pub(super) fn build(
        case: &'static str,
        extra: &str,
        lane_tick: Option<Duration>,
    ) -> Option<Self> {
        let herdr = IsolatedHerdr::new(case)?;
        let root = herdr.root().to_path_buf();
        fs::create_dir_all(root.join("cfg")).unwrap();
        fs::write(root.join("cfg/herdr.toml"), "default_shell = '/bin/sh'\n").unwrap();
        let smoke = Smoke {
            state: root.join("plugin-state"),
            cwd: root.join("home"),
            herdr,
            labels: 0,
            processes: Vec::new(),
            lane_tick,
        };
        fs::create_dir_all(&smoke.cwd).unwrap();
        smoke.write_settings(extra);
        smoke.herdr.start();
        smoke.human(None, &["daemon", "ensure"]).data();
        Some(smoke)
    }

    /// `{"harness_manifest":"off"}` before anything starts the daemon, so harness admission does
    /// not depend on the embedded manifest (as the Hermes smoke does).
    pub(super) fn write_settings(&self, extra: &str) {
        let paths = self.paths();
        paths.prepare_node_dir().unwrap();
        let settings = paths
            .node_dir
            .join(herdr_threads::daemon::settings::SETTINGS_FILE);
        fs::write(&settings, format!(r#"{{{extra}"harness_manifest":"off"}}"#)).unwrap();
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
    }

    pub(super) fn paths(&self) -> herdr_threads::daemon::paths::NodePaths {
        let runtime = herdr_threads::daemon::paths::RuntimeContext::explicit(
            self.state.clone(),
            self.herdr.socket_path(),
            None,
        )
        .unwrap();
        herdr_threads::daemon::paths::NodePaths::resolve(&runtime.state_dir).unwrap()
    }

    /// A new long-lived process to run an agent in: its pid is the agent's liveness fingerprint.
    pub(super) fn stand_in(&mut self) -> i64 {
        let child = Command::new("/bin/sleep")
            .arg("86400")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn_owned()
            .unwrap();
        let pid = i64::from(child.id());
        self.processes.push(child);
        pid
    }

    /// Stops the stand-in started by `stand_in` whose pid is `pid` (the agent's process exits).
    pub(super) fn end_stand_in(&mut self, pid: i64) {
        let index = self
            .processes
            .iter()
            .position(|child| i64::from(child.id()) == pid)
            .expect("a stand-in with that pid");
        self.processes.remove(index).stop();
    }

    pub(super) fn routing(&self) -> Vec<String> {
        [
            "--state-dir",
            self.state.to_str().unwrap(),
            "--host-endpoint",
            self.herdr.socket_path().to_str().unwrap(),
        ]
        .map(String::from)
        .to_vec()
    }

    /// The private environment every child gets: scrubbed, isolated HOME and config homes under
    /// the Herdr root, never the real ones.
    pub(super) fn private(&self, command: &mut Command, pane: Option<&str>) {
        let root = self.herdr.root();
        scrub_env(command);
        command.envs(self.herdr.env());
        command
            .env("CLAUDE_CONFIG_DIR", root.join("claude"))
            .env("CODEX_HOME", root.join("codex"))
            .env("HERMES_HOME", root.join("hermes"))
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env_remove("HERDR_ENV")
            .env_remove("HERDR_PANE_ID");
        if let Some(pane) = pane {
            command.env("HERDR_ENV", "1").env("HERDR_PANE_ID", pane);
        }
        // Read only by the daemon a call starts (test-support builds).
        if let Some(tick) = self.lane_tick {
            command.env(
                herdr_threads::service::config::TEST_LANE_TICK_ENV,
                tick.as_millis().to_string(),
            );
        }
    }

    pub(super) fn finish(mut command: Command, stdin: Option<&[u8]>) -> Out {
        command
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn_owned().unwrap();
        if let Some(bytes) = stdin {
            child.stdin.take().unwrap().write_all(bytes).unwrap();
        }
        Out::of(child.wait_with_output().unwrap())
    }

    /// A person (no harness parent, no harness markers), `--json`, optionally in `pane`.
    pub(super) fn human(&self, pane: Option<&str>, args: &[&str]) -> Out {
        let mut command = spawn::command(BIN);
        self.private(&mut command, pane);
        // `human` goes right after the executable, before the routing flags.
        let (namespace, rest) = match args.split_first() {
            Some((&"human", rest)) => (vec!["human"], rest),
            _ => (Vec::new(), args),
        };
        command
            .args(namespace)
            .arg("--json")
            .args(self.routing())
            .args(rest);
        Self::finish(command, None)
    }

    /// The command line of an agent's call: `argv` is everything after the executable.
    pub(super) fn agent_command(&self, who: &Agent, argv: &[String]) -> Command {
        let mut command = if who.shim {
            let args: Vec<&str> = argv.iter().map(String::as_str).collect();
            if who.managed_daemon {
                harness_shim::under_codex_managed_daemon(Path::new(BIN), &args, &[])
            } else {
                harness_shim::under_harness(who.harness, Path::new(BIN), &args, &[])
            }
        } else {
            let mut command = spawn::command(BIN);
            command.args(argv);
            command
        };
        self.private(&mut command, Some(&who.pane));
        for (variable, value) in &who.inherited {
            command.env(variable, value);
        }
        if who.shim {
            if !who.no_session {
                command.env(who.session_variable(), &who.session);
            }
        } else {
            as_agent(&mut command, who.harness, &who.session);
            if let Some(pid) = who.process {
                command
                    .env("CLAUDE_PID", pid.to_string())
                    .env("HT_TEST_HARNESS_PID", pid.to_string());
            }
        }
        if let Some(thread) = &who.thread {
            command.env("CODEX_THREAD_ID", thread);
        }
        command
    }

    /// An agent's CLI call, `--json`.
    pub(super) fn cli(&self, who: &Agent, args: &[&str]) -> Out {
        let argv: Vec<String> = ["--json".to_owned()]
            .into_iter()
            .chain(self.routing())
            .chain(args.iter().map(|a| (*a).to_owned()))
            .collect();
        Self::finish(self.agent_command(who, &argv), None)
    }

    /// An agent's CLI call in the default text format: the route that records body display (and
    /// so advances the caller's own cursor); `--json` reads are passive.
    pub(super) fn text(&self, who: &Agent, args: &[&str]) -> Out {
        let argv: Vec<String> = self
            .routing()
            .into_iter()
            .chain(args.iter().map(|a| (*a).to_owned()))
            .collect();
        Self::finish(self.agent_command(who, &argv), None)
    }

    /// `herdr-threads hook <harness>` fed one native payload.
    pub(super) fn hook(&self, who: &Agent, payload: &Value) -> Out {
        let argv: Vec<String> = self
            .routing()
            .into_iter()
            .chain(["hook".to_owned(), who.harness.to_owned()])
            .collect();
        let command = self.agent_command(who, &argv);
        Self::finish(command, Some(&serde_json::to_vec(payload).unwrap()))
    }

    /// The production owned native entrypoint, with its declared event registration.
    pub(super) fn native_hook(&self, who: &Agent, payload: &Value) -> Out {
        let argv: Vec<String> = self
            .routing()
            .into_iter()
            .chain([
                "hook-event".to_owned(),
                who.harness.to_owned(),
                payload["hook_event_name"].as_str().unwrap().to_owned(),
            ])
            .collect();
        Self::finish(
            self.agent_command(who, &argv),
            Some(&serde_json::to_vec(payload).unwrap()),
        )
    }

    /// A fresh `herdr workspace create` root pane (never a pane we did not make).
    pub(super) fn pane(&mut self) -> String {
        self.labels += 1;
        let label = format!("bindings-smoke-{}", self.labels);
        let out = self
            .herdr
            .command("herdr")
            .args([
                "workspace",
                "create",
                "--label",
                &label,
                "--no-focus",
                "--cwd",
            ])
            .arg(&self.cwd)
            .output()
            .unwrap();
        assert!(out.status.success(), "workspace create: {out:?}");
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        value["result"]["root_pane"]["pane_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    pub(super) fn herdr_api(&self, args: &[&str]) -> Value {
        let output = self.herdr.command("herdr").args(args).output().unwrap();
        assert!(output.status.success(), "herdr {args:?} failed: {output:?}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        value["result"].clone()
    }

    pub(super) fn seat(&self, pane: &str) -> String {
        self.human(None, &["seat", "resolve", "--pane", pane])
            .data()
            .as_str()
            .unwrap()
            .to_owned()
    }

    pub(super) fn db(&self) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(self.paths().database_path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }

    pub(super) fn count(&self, sql: &str, params: &[&dyn rusqlite::ToSql]) -> i64 {
        self.db().query_row(sql, params, |row| row.get(0)).unwrap()
    }

    /// Every binding row of the seat, oldest first; ended rows included.
    pub(super) fn all_bindings(&self, seat: &str) -> Vec<Binding> {
        let db = self.db();
        let mut statement = db
            .prepare(
                "SELECT binding_id,harness,native_session,role,agent_id,liveness,disconnect_reason,\
                 observation_provenance,liveness_pid,ended_at IS NOT NULL,parent_binding \
                 FROM occupant_bindings WHERE seat_id=?1 ORDER BY ordinal",
            )
            .unwrap();
        statement
            .query_map([seat], |r| {
                Ok(Binding {
                    id: r.get(0)?,
                    harness: r.get(1)?,
                    session: r.get(2)?,
                    role: r.get(3)?,
                    agent_id: r.get(4)?,
                    liveness: r.get(5)?,
                    reason: r.get(6)?,
                    provenance: r.get(7)?,
                    pid: r.get(8)?,
                    ended: r.get(9)?,
                    parent: r.get(10)?,
                })
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// The seat's open (not ended) bindings.
    pub(super) fn bindings(&self, seat: &str) -> Vec<Binding> {
        self.all_bindings(seat)
            .into_iter()
            .filter(|b| !b.ended)
            .collect()
    }

    pub(super) fn binding(&self, seat: &str, harness: &str, session: &str, agent: &str) -> Binding {
        let found: Vec<Binding> = self
            .bindings(seat)
            .into_iter()
            .filter(|b| b.harness == harness && b.session == session && b.agent_id == agent)
            .collect();
        assert_eq!(
            found.len(),
            1,
            "one open ({harness}, {session}, {agent:?}) binding: {:?}",
            self.all_bindings(seat)
        );
        found[0].clone()
    }

    pub(super) fn cursor(&self, binding: &str, thread: &str) -> Option<i64> {
        self.db()
            .query_row(
                "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
                [binding, thread],
                |r| r.get(0),
            )
            .ok()
    }

    /// Every row of `table`, for a failure message.
    pub(super) fn dump(&self, table: &str) -> String {
        let db = self.db();
        let mut statement = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let columns: Vec<String> = statement
            .column_names()
            .iter()
            .map(|c| c.to_string())
            .collect();
        statement
            .query_map([], |row| {
                Ok((0..columns.len())
                    .map(|i| format!("{}={:?}", columns[i], row.get_ref(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join(" "))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .collect::<Vec<_>>()
            .join(" | ")
    }

    pub(super) fn head(&self, thread: &str) -> i64 {
        self.count(
            "SELECT COALESCE(MAX(sequence),0) FROM messages WHERE thread_id=?1",
            &[&thread],
        )
    }
}

impl Drop for Smoke {
    fn drop(&mut self) {
        // Every daemon this smoke (or a hook it ran) started stops before the test ends; the
        // IsolatedHerdr teardown that follows stops the server.
        let mut command = spawn::command(BIN);
        self.private(&mut command, None);
        let _ = command
            .args(self.routing())
            .args(["daemon", "stop"])
            .output();
    }
}

// ---- synthetic native payloads ---------------------------------------------------------------

pub(super) fn claude_start(session: &str, source: &str) -> Value {
    json!({
        "hook_event_name": "SessionStart", "session_id": session,
        "transcript_path": "/tmp/t.jsonl", "cwd": "/tmp", "source": source,
    })
}

pub(super) fn claude_subagent(event: &str, session: &str, agent: &str) -> Value {
    json!({
        "hook_event_name": event, "session_id": session,
        "agent_id": agent, "agent_type": "worker",
    })
}

pub(super) fn claude_tool(session: &str, command: &str, agent: Option<&str>) -> Value {
    let mut payload = json!({
        "hook_event_name": "PreToolUse", "session_id": session,
        "transcript_path": "/tmp/t.jsonl", "cwd": "/tmp", "permission_mode": "default",
        "tool_name": "Bash", "tool_input": { "command": command },
        "tool_use_id": format!("toolu_{}", uuid::Uuid::new_v4().simple()),
    });
    if let Some(agent) = agent {
        payload["agent_id"] = agent.into();
        payload["agent_type"] = "worker".into();
    }
    payload
}

fn codex_start(session: &str, source: &str) -> Value {
    json!({
        "hook_event_name": "SessionStart", "session_id": session, "source": source,
        "turn_id": "turn-1",
    })
}

fn codex_subagent_start(session: &str, agent: &str) -> Value {
    json!({
        "hook_event_name": "SubagentStart", "session_id": session, "turn_id": "turn-1",
        "cwd": "/tmp", "model": "m", "permission_mode": "default",
        "agent_id": agent, "agent_type": "worker", "transcript_path": null,
    })
}

/// A Hermes bridge envelope for a row of `enrollment-envelopes.json` with fresh timestamps and
/// a strictly increasing observation sequence (the shape `hermes_enrollment_sweep` drives).
fn hermes_envelope(row: &str, session: &str, event: &str) -> Value {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let rows: serde_json::Map<String, Value> = serde_json::from_slice(include_bytes!(
        "../fixtures/hermes/enrollment-envelopes.json"
    ))
    .unwrap();
    let mut payload = rows[row].clone();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    payload["session_id"] = session.into();
    payload["event_id"] = event.into();
    if !payload["turn_id"].is_null() {
        payload["turn_id"] = event.into();
    }
    if payload["role_association"].is_object() {
        payload["role_association"]["session_id"] = session.into();
        if !payload["role_association"]["turn_id"].is_null() {
            payload["role_association"]["turn_id"] = event.into();
        }
    }
    payload["started_at"] = now.into();
    payload["deadline_at"] = (now + 1200).into();
    payload["observation_order"]["observed_at_millis"] = now.into();
    payload["observation_order"]["sequence"] =
        (SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1).into();
    payload
}

// ---- configuration 1 -----------------------------------------------------------------------

/// A thread `owner` created and `member` joined, `member` already enrolled so every one of its
/// bindings is seeded at the join point.
pub(super) fn joined_thread(
    smoke: &Smoke,
    owner: &Agent,
    member: &Agent,
    member_seat: &str,
) -> String {
    let thread = smoke
        .cli(owner, &["thread", "create", "--topic", "smoke"])
        .data()
        .as_str()
        .unwrap()
        .to_owned();
    smoke
        .cli(owner, &["invite", &thread, "--seat", member_seat])
        .data();
    smoke.cli(member, &["accept", &thread]).data();
    thread
}

// Kills: bindings keyed by seat (a subagent or a second reader sharing the main session's
// cursor), subagent writes served as the parent's, and a PreToolUse rewrite that is not in the
// installed hook's output.
#[test]
fn config1_claude_main_and_two_subagents() {
    let Some(mut smoke) = Smoke::ticking("config1_claude_main_and_two_subagents") else {
        return;
    };
    let (pane, peer_pane) = (smoke.pane(), smoke.pane());
    let (seat, peer_seat) = (smoke.seat(&pane), smoke.seat(&peer_pane));
    let main = Agent::claude("sess-main", &pane);
    let peer = Agent::claude("sess-peer", &peer_pane);
    for who in [&main, &peer] {
        let out = smoke.hook(who, &claude_start(&who.session, "startup"));
        assert_eq!(out.code, 0, "{}", out.text());
    }
    // Two subagents enroll through SubagentStart.
    for agent in ["agent-1", "agent-2"] {
        let out = smoke.hook(&main, &claude_subagent("SubagentStart", "sess-main", agent));
        assert_eq!(out.code, 0, "{}", out.text());
    }
    let open = smoke.bindings(&seat);
    assert_eq!(open.len(), 3, "{open:?}");
    let top = smoke.binding(&seat, "claude", "sess-main", "");
    let (one, two) = (
        smoke.binding(&seat, "claude", "sess-main", "agent-1"),
        smoke.binding(&seat, "claude", "sess-main", "agent-2"),
    );
    assert_eq!(top.role, "top_level");
    for sub in [&one, &two] {
        assert_eq!(sub.role, "subagent", "{sub:?}");
        assert_eq!(sub.parent.as_deref(), Some(top.id.as_str()), "{sub:?}");
    }
    assert_ne!(one.id, two.id);

    // Mail arrives; each reader's own display moves only its own cursor.
    let thread = joined_thread(&smoke, &peer, &main, &seat);
    // The join leaves its own marker after the cursors: the join point is where they all stand.
    let join_point = smoke
        .cursor(&top.id, &thread)
        .expect("the main binding has a cursor in the joined thread");
    for body in ["first", "second"] {
        smoke.cli(&peer, &["send", &thread, "--body", body]).data();
    }
    let head = smoke.head(&thread);
    assert!(head >= join_point + 2, "{head} {join_point}");
    for binding in [&top, &one, &two] {
        assert_eq!(
            smoke.cursor(&binding.id, &thread),
            Some(join_point),
            "{binding:?} starts at the join point"
        );
    }
    // A subagent's passive read sees the mail (through the hook-inserted `--agent-id`) and moves
    // no cursor; the main session's display moves only the main cursor.
    let passive = smoke.cli(&main, &["--agent-id=agent-1", "inbox"]);
    let items = passive.data()["items"].clone();
    assert_eq!(items.as_array().map(Vec::len), Some(2), "{items}");
    for binding in [&top, &one, &two] {
        assert_eq!(smoke.cursor(&binding.id, &thread), Some(join_point));
    }
    let read = smoke.text(&main, &["inbox"]);
    assert_eq!(read.code, 0, "{}", read.text());
    assert_eq!(smoke.cursor(&top.id, &thread), Some(head), "main read");
    for binding in [&one, &two] {
        assert_eq!(
            smoke.cursor(&binding.id, &thread),
            Some(join_point),
            "{binding:?}: the main session's display leaves a subagent's cursor alone"
        );
    }

    // Subagents are read-only: a send and an ack are refused, nothing is attributed to them.
    let sent = smoke.cli(
        &main,
        &[
            "--agent-id=agent-2",
            "send",
            &thread,
            "--body",
            "from a subagent",
        ],
    );
    assert_eq!(sent.code, REFUSED, "{}", sent.text());
    assert!(sent.text().contains("subagent"), "{}", sent.text());
    let acked = smoke.cli(&main, &["--agent-id=agent-2", "ack", &thread]);
    assert_eq!(acked.code, REFUSED, "{}", acked.text());
    assert!(acked.text().contains("subagent"), "{}", acked.text());
    assert_eq!(
        smoke.count(
            "SELECT COUNT(*) FROM messages WHERE thread_id=?1 AND body LIKE '%from a subagent%'",
            &[&thread]
        ),
        0
    );

    // The installed PreToolUse hook rewrites a subagent's `ht` call, and only that.
    let rewritten = smoke.hook(
        &main,
        &claude_tool(
            "sess-main",
            "cd x && ht inbox | ht send t m",
            Some("agent-1"),
        ),
    );
    assert_eq!(rewritten.code, 0, "{}", rewritten.text());
    let specific = rewritten.json()["hookSpecificOutput"].clone();
    assert_eq!(
        specific["updatedInput"]["command"],
        "cd x && ht --agent-id=agent-1 inbox | ht --agent-id=agent-1 send t m",
        "{specific}"
    );
    assert!(specific.get("permissionDecision").is_none(), "{specific}");
    let plain = smoke.hook(&main, &claude_tool("sess-main", "ht inbox", None));
    assert_eq!(plain.code, 0, "{}", plain.text());
    assert!(!plain.stdout.contains("updatedInput"), "{}", plain.stdout);

    // Cross-pane resume: the same session id in a second pane is a new binding on that pane's
    // seat; the first binding row is untouched.
    let before = smoke.all_bindings(&seat);
    let other_pane = smoke.pane();
    let other_seat = smoke.seat(&other_pane);
    let resumed = Agent::claude("sess-main", &other_pane).in_process(smoke.stand_in());
    let out = smoke.hook(&resumed, &claude_start("sess-main", "resume"));
    assert_eq!(out.code, 0, "{}", out.text());
    let moved = smoke.binding(&other_seat, "claude", "sess-main", "");
    assert_ne!(moved.id, top.id);
    assert_eq!(moved.role, "top_level");
    assert_eq!(moved.liveness, "connected", "{moved:?}");
    assert_eq!(
        smoke.all_bindings(&seat),
        before,
        "the first pane's bindings are untouched"
    );
    let _ = peer_seat;

    // /clear in the main pane: the new session's SessionStart supersedes the old binding (same
    // harness process). A stale caller that still carries the pre-/clear session id is served as
    // the old, superseded binding and never reconnects it (ht-54r.87): the old binding stays
    // disconnected, the live one stays connected, and nothing writes `liveness`, so the archival
    // mutation revision does not churn, now or after one liveness cadence.
    let process = smoke.binding(&seat, "claude", "sess-main", "");
    let cleared = Agent::claude("sess-clear", &pane).in_process_of(&process);
    let out = smoke.hook(&cleared, &claude_start("sess-clear", "clear"));
    assert_eq!(out.code, 0, "{}", out.text());
    let old = smoke
        .all_bindings(&seat)
        .into_iter()
        .find(|b| b.id == top.id)
        .unwrap();
    assert_eq!(old.liveness, "disconnected", "{old:?}");
    let live = smoke.binding(&seat, "claude", "sess-clear", "");
    assert_eq!(live.liveness, "connected", "{live:?}");
    let revision = || smoke.count("SELECT mutation_revision FROM archival_instances", &[]);
    // A `thread create` is the call that resolves and writes through the caller's binding, but it
    // bumps the archival revision itself. The live session's own create is the control: the stale
    // caller's create must bump it by exactly the same amount, with no `liveness` write on top.
    let control_before = revision();
    let control = smoke.cli(&cleared, &["thread", "create", "--topic", "live caller"]);
    assert_eq!(control.code, 0, "{}", control.text());
    let create_bump = revision() - control_before;
    let stale = Agent::claude("sess-main", &pane).in_process_of(&process);
    let revision_before = revision();
    let served = smoke.cli(&stale, &["thread", "create", "--topic", "stale caller"]);
    assert_eq!(served.code, 0, "{}", served.text());
    let revision_after_call = revision();
    assert_eq!(
        revision_after_call - revision_before,
        create_bump,
        "the stale call wrote liveness on top of its thread create"
    );
    let still_settled = || {
        let after = smoke.all_bindings(&seat);
        let old_after = after.iter().find(|b| b.id == top.id).unwrap();
        assert_eq!(old_after.liveness, "disconnected", "{old_after:?}");
        assert_eq!(old_after.reason, old.reason, "{old_after:?}");
        assert_eq!(
            after.iter().find(|b| b.id == live.id).unwrap().liveness,
            "connected",
            "the live binding does not flap"
        );
    };
    still_settled();
    // One liveness cadence later (the worker's next pass): the same three facts, and no churn.
    // The window is 6/5 of the cadence (6 s at the production 5 s).
    let until = Instant::now() + SMOKE_TICK * 6 / 5;
    while Instant::now() < until {
        still_settled();
        assert_eq!(
            revision(),
            revision_after_call,
            "a stale call or a worker pass wrote liveness"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

// Kills: a subagent's text inbox that cannot display or advance its own cursor (fixed by ht-54r.58).
#[test]
fn config1_subagent_display_advances_its_own_cursor() {
    let Some(mut smoke) = Smoke::new("config1_subagent_display_advances_its_own_cursor") else {
        return;
    };
    let (pane, peer_pane) = (smoke.pane(), smoke.pane());
    let seat = smoke.seat(&pane);
    smoke.seat(&peer_pane);
    let main = Agent::claude("sess-main", &pane);
    let peer = Agent::claude("sess-peer", &peer_pane);
    for who in [&main, &peer] {
        assert_eq!(
            smoke.hook(who, &claude_start(&who.session, "startup")).code,
            0
        );
    }
    let out = smoke.hook(
        &main,
        &claude_subagent("SubagentStart", "sess-main", "agent-1"),
    );
    assert_eq!(out.code, 0, "{}", out.text());
    let thread = joined_thread(&smoke, &peer, &main, &seat);
    smoke
        .cli(&peer, &["send", &thread, "--body", "hello"])
        .data();
    let sub = smoke.binding(&seat, "claude", "sess-main", "agent-1");
    let read = smoke.text(&main, &["--agent-id=agent-1", "inbox"]);
    assert_eq!(read.code, 0, "{}", read.text());
    assert_eq!(smoke.cursor(&sub.id, &thread), Some(smoke.head(&thread)));
}

// ---- configuration 2 -----------------------------------------------------------------------

// Kills: a Codex subagent keyed by the root session alone (collapsing into the parent), a thread
// id taken for a session, a CLI path that disagrees with the hook path about the key, and a
// managed-daemon process admitted as an ordinary Codex.
#[test]
fn config2_codex_embedded_and_managed_daemon() {
    let Some(mut smoke) = Smoke::new("config2_codex_embedded_and_managed_daemon") else {
        return;
    };
    let pane = smoke.pane();
    let seat = smoke.seat(&pane);
    // Embedded: thread == session is the top level.
    let root = Agent::codex("codex-root", &pane).thread("codex-root");
    let out = smoke.hook(&root, &codex_start("codex-root", "startup"));
    assert_eq!(out.code, 0, "{}", out.text());
    let top = smoke.binding(&seat, "codex", "codex-root", "");
    assert_eq!(top.role, "top_level", "{top:?}");
    // A thread that differs from the session is a subagent keyed (session, thread), on the hook
    // path (SubagentStart carries the root session and the child's agent id) ...
    let out = smoke.hook(&root, &codex_subagent_start("codex-root", "thread-hook"));
    assert_eq!(out.code, 0, "{}", out.text());
    let hooked = smoke.binding(&seat, "codex", "codex-root", "thread-hook");
    assert_eq!(hooked.role, "subagent", "{hooked:?}");
    assert_eq!(hooked.parent.as_deref(), Some(top.id.as_str()));
    // ... and on the CLI path: CODEX_THREAD_ID differing from CODEX_SESSION_ID is that subagent
    // (a write is refused for its role, and no second binding appears), while the root's own
    // thread id is the top level (the same write is served).
    let child = Agent::codex("codex-root", &pane).thread("thread-hook");
    let write = smoke.cli(
        &child,
        &["thread", "create", "--topic", "from a child thread"],
    );
    assert_eq!(write.code, REFUSED, "{}", write.text());
    assert!(write.text().contains("subagent"), "{}", write.text());
    let write = smoke.cli(
        &root,
        &["thread", "create", "--topic", "from the root thread"],
    );
    assert_eq!(write.code, 0, "{}", write.text());
    assert_eq!(
        smoke.bindings(&seat).len(),
        2,
        "{:?}",
        smoke.bindings(&seat)
    );
    assert_eq!(smoke.binding(&seat, "codex", "codex-root", "").id, top.id);
    assert_eq!(
        smoke
            .binding(&seat, "codex", "codex-root", "thread-hook")
            .id,
        hooked.id
    );

    // A process under `codex app-server --managed-daemon` is refused with the pane hint, before
    // any daemon call and without enrolling anything.
    let managed = Agent::codex("codex-managed", &pane).under_managed_daemon();
    let refused = smoke.cli(&managed, &["inbox"]);
    assert_eq!(refused.code, REFUSED, "{}", refused.text());
    for needle in ["managed_daemon_refused", "codex-daemon-env", "--no-daemon"] {
        assert!(
            refused.text().contains(needle),
            "{needle}: {}",
            refused.text()
        );
    }
    let hooked = smoke.hook(&managed, &codex_start("codex-managed", "startup"));
    // A hook never fails its harness: it exits 0 and says why on stderr.
    assert!(hooked.stderr.contains("--no-daemon"), "{}", hooked.text());
    assert_eq!(smoke.bindings(&seat).len(), 2, "nothing was enrolled");
}

// ---- configuration 3 -----------------------------------------------------------------------

impl Smoke {
    /// One Hermes callback through `hook hermes`. A turn must yield context; the envelope carries
    /// the production 1200 ms cap, so only an overrun (an empty answer after at least a second)
    /// is replayed as the same event with fresh timestamps.
    fn hermes(&self, who: &Agent, row: &str, event: &str) -> Value {
        let mut out = Out {
            code: 0,
            stdout: String::new(),
            stderr: String::new(),
        };
        for _ in 0..4 {
            let started = Instant::now();
            out = self.hook(who, &hermes_envelope(row, &who.session, event));
            assert_eq!(out.code, 0, "{row}: {}", out.text());
            if row != "top_turn_cli"
                || !out.stdout.trim().is_empty()
                || started.elapsed() < Duration::from_secs(1)
            {
                break;
            }
        }
        if out.stdout.trim().is_empty() {
            Value::Null
        } else {
            out.json()
        }
    }
}

// Kills: a one-shot taking the outer Claude's session (inherited env) or the outer binding's
// identity, a Claude binding that stops receiving notices or ACKing beside a Hermes one, sends
// attributed to the seat instead of the binding, a one-shot that stays connected after its
// process is gone, and the per-turn `on_session_end` disconnecting the interactive Hermes.
#[test]
fn config3_hermes_interactive_and_a_one_shot_in_a_claude_pane() {
    let Some(mut smoke) =
        Smoke::ticking("config3_hermes_interactive_and_a_one_shot_in_a_claude_pane")
    else {
        return;
    };
    let (pane, peer_pane) = (smoke.pane(), smoke.pane());
    let (seat, peer_seat) = (smoke.seat(&pane), smoke.seat(&peer_pane));
    let claude = Agent::claude("sess-claude", &pane);
    let peer = Agent::claude("sess-peer", &peer_pane);
    for who in [&claude, &peer] {
        assert_eq!(
            smoke.hook(who, &claude_start(&who.session, "startup")).code,
            0
        );
    }
    let thread = joined_thread(&smoke, &peer, &claude, &seat);

    // An interactive Hermes in the same pane: start, then its first turn.
    let hermes = Agent::hermes("herm-1", &pane);
    assert!(smoke.hermes(&hermes, "cli_start", "start-1").is_null());
    let first = smoke.hermes(&hermes, "top_turn_cli", "turn-1");
    assert!(
        first["context"].is_string(),
        "the first turn delivers context: {first}"
    );
    let claude_binding = smoke.binding(&seat, "claude", "sess-claude", "");
    let hermes_binding = smoke.binding(&seat, "hermes", "herm-1", "");
    assert_eq!(hermes_binding.role, "top_level");
    assert_eq!(hermes_binding.liveness, "connected");

    // `hermes -z` from Claude's Bash tool: a Hermes process (nearest harness) whose environment
    // still carries Claude's session variable. It gets its own Hermes binding.
    let one_shot = Agent::hermes("herm-z", &pane)
        .shimmed()
        .inheriting("CLAUDE_CODE_SESSION_ID", "sess-claude");
    // Its first call is a write (a read never enrolls), so the binding exists once it returns.
    let one_shot_sent = smoke
        .cli(&one_shot, &["send", &thread, "--body", "from herm-z"])
        .data()
        .as_str()
        .unwrap()
        .to_owned();
    let z_binding = smoke.binding(&seat, "hermes", "herm-z", "");
    assert_eq!(z_binding.role, "top_level", "{z_binding:?}");
    assert_eq!(z_binding.provenance, "cooperative_env_session");
    assert_ne!(z_binding.id, hermes_binding.id);
    assert_ne!(z_binding.id, claude_binding.id);
    assert_eq!(
        smoke
            .bindings(&seat)
            .iter()
            .filter(|b| b.harness == "claude")
            .count(),
        1,
        "the inherited Claude variable made no Claude binding"
    );
    assert_eq!(
        smoke.binding(&seat, "claude", "sess-claude", "").liveness,
        "connected",
        "Claude's binding is untouched by the one-shot"
    );

    // Claude keeps its notices and its ACKs: an ACK-required message reaches the Claude hook,
    // and Claude's own display settles it, recorded against Claude's binding.
    let message = smoke
        .cli(
            &peer,
            &[
                "send",
                &thread,
                "--body",
                "please ack",
                "--require-ack",
                &seat,
            ],
        )
        .data()
        .as_str()
        .unwrap()
        .to_owned();
    let notice = smoke.hook(&claude, &claude_tool("sess-claude", "ls", None));
    assert_eq!(notice.code, 0, "{}", notice.text());
    assert!(
        notice.stdout.contains(&thread) || notice.stdout.contains(&message),
        "Claude's tool hook still carries the notice: {}",
        notice.stdout
    );
    // Complete hook display settles the receipt without a separate ACK call.
    let acker: Option<String> = smoke
        .db()
        .query_row(
            "SELECT ack_binding_id FROM receipt_state WHERE message_id=?1 AND seat_id=?2 AND state='acked'",
            [&message, &seat],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(acker.as_deref(), Some(claude_binding.id.as_str()));
    let observation: String = smoke
        .db()
        .query_row(
            "SELECT ack_observation FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
            [&message, &seat],
            |r| r.get(0),
        )
        .unwrap();
    let observation: serde_json::Value = serde_json::from_str(&observation).unwrap();
    assert_eq!(observation["action_provenance"], "cooperative_hook_context");

    // Sends are attributed per binding (the one-shot's was sent above).
    for (who, binding) in [(&claude, &claude_binding), (&hermes, &hermes_binding)] {
        let sent = smoke
            .cli(
                who,
                &["send", &thread, "--body", &format!("from {}", who.session)],
            )
            .data()
            .as_str()
            .unwrap()
            .to_owned();
        let sender: Option<String> = smoke
            .db()
            .query_row(
                "SELECT sender_binding_id FROM messages WHERE id=?1",
                [&sent],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            sender.as_deref(),
            Some(binding.id.as_str()),
            "{}",
            who.session
        );
    }
    let sender: Option<String> = smoke
        .db()
        .query_row(
            "SELECT sender_binding_id FROM messages WHERE id=?1",
            [&one_shot_sent],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        sender.as_deref(),
        Some(z_binding.id.as_str()),
        "the one-shot"
    );

    // The one-shot's process is gone: the liveness worker disconnects exactly that binding.
    wait_until(
        "the one-shot to disconnect",
        Duration::from_secs(30),
        || smoke.binding(&seat, "hermes", "herm-z", "").liveness == "disconnected",
    );
    let z_after = smoke.binding(&seat, "hermes", "herm-z", "");
    assert_eq!(
        z_after.reason.as_deref(),
        Some("liveness_process_exit"),
        "{z_after:?}"
    );
    for (harness, session) in [("claude", "sess-claude"), ("hermes", "herm-1")] {
        assert_eq!(
            smoke.binding(&seat, harness, session, "").liveness,
            "connected",
            "{session} survives the one-shot's exit"
        );
    }

    // Between turns: Hermes fires `on_session_end` after every turn. It is not an end event.
    let between = smoke.hook(
        &hermes,
        &hermes_envelope("session_finalize", "herm-1", "end-1")
            .tap(|v| v["callback"] = "on_session_end".into()),
    );
    assert_eq!(between.code, 0, "{}", between.text());
    assert_eq!(
        smoke.binding(&seat, "hermes", "herm-1", "").liveness,
        "connected",
        "on_session_end must not disconnect the interactive binding"
    );
    // Its teardown, `on_session_finalize`, is.
    smoke.hermes(&hermes, "session_finalize", "finalize-1");
    let ended = smoke.binding(&seat, "hermes", "herm-1", "");
    assert_eq!(ended.liveness, "disconnected", "{ended:?}");
    assert_eq!(
        ended.reason.as_deref(),
        Some("liveness_end_event"),
        "{ended:?}"
    );
    assert_eq!(
        smoke.binding(&seat, "claude", "sess-claude", "").liveness,
        "connected"
    );
    let _ = peer_seat;

    // The wake lane still targets an interactive Hermes binding after its per-turn
    // `on_session_end`, and stops once its teardown disconnects it. A pane of its own, so the
    // Hermes binding is the only connected one; a shell pane never accepts the nudge itself
    // (`unsafe`), but each attempt moves `wake_work.completed_at_utc`.
    let lone_pane = smoke.pane();
    let lone_seat = smoke.seat(&lone_pane);
    let lone = Agent::hermes("herm-lone", &lone_pane);
    smoke.hermes(&lone, "cli_start", "start-lone");
    assert!(smoke.hermes(&lone, "top_turn_cli", "turn-lone")["context"].is_string());
    let lone_thread = joined_thread(&smoke, &peer, &lone, &lone_seat);
    let attempted = |at: i64| {
        smoke
            .db()
            .query_row(
                "SELECT COALESCE(completed_at_utc,0) FROM wake_work WHERE seat_id=?1",
                [&lone_seat],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > at
    };
    let last_attempt = || {
        smoke
            .db()
            .query_row(
                "SELECT COALESCE(completed_at_utc,0) FROM wake_work WHERE seat_id=?1",
                [&lone_seat],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
    };
    let between = smoke.hook(
        &lone,
        &hermes_envelope("session_finalize", "herm-lone", "end-lone")
            .tap(|v| v["callback"] = "on_session_end".into()),
    );
    assert_eq!(between.code, 0, "{}", between.text());
    let mark = last_attempt();
    smoke
        .cli(
            &peer,
            &[
                "send",
                &lone_thread,
                "--body",
                "wake",
                "--require-ack",
                &lone_seat,
            ],
        )
        .data();
    wait_until(
        "a wake attempt on the Hermes binding",
        Duration::from_secs(20),
        || attempted(mark),
    );
    smoke.hermes(&lone, "session_finalize", "finalize-lone");
    // An attempt reserved before the teardown may still complete (as unsafe)
    // after it; on a short lane tick a retry of the first send can be that
    // attempt. Let it settle, then no new attempt may be reserved or completed.
    let in_flight = || {
        smoke
            .db()
            .query_row(
                "SELECT reservation_id IS NOT NULL FROM wake_work WHERE seat_id=?1",
                [&lone_seat],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(false)
    };
    let last_reserved = || {
        smoke
            .db()
            .query_row(
                "SELECT COALESCE(last_reserved_at_utc,0) FROM wake_work WHERE seat_id=?1",
                [&lone_seat],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
    };
    wait_until(
        "the lone seat's in-flight wake attempt to settle",
        Duration::from_secs(20),
        || !in_flight(),
    );
    let mark = last_attempt();
    let reserved_mark = last_reserved();
    smoke
        .cli(
            &peer,
            &[
                "send",
                &lone_thread,
                "--body",
                "again",
                "--require-ack",
                &lone_seat,
            ],
        )
        .data();
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        !attempted(mark) && !in_flight() && last_reserved() == reserved_mark,
        "a disconnected binding is not a wake target"
    );
}

trait Tap: Sized {
    fn tap(self, edit: impl FnOnce(&mut Self)) -> Self;
}
impl Tap for Value {
    fn tap(mut self, edit: impl FnOnce(&mut Self)) -> Self {
        edit(&mut self);
        self
    }
}

// ---- configuration 4 -----------------------------------------------------------------------

// Kills: `me init` over a connected agent (two identities on one pane), a human binding that
// survives an agent's check-in, a successful `me init` that leaves the disconnected agent binding
// open, and a resume that reopens a binding the exclusivity transition ended.
#[test]
fn config4_human_pane_exclusivity() {
    let Some(mut smoke) = Smoke::new("config4_human_pane_exclusivity") else {
        return;
    };
    let pane = smoke.pane();
    let seat = smoke.seat(&pane);
    let codex = Agent::codex("codex-a", &pane).thread("codex-a");
    let out = smoke.hook(&codex, &codex_start("codex-a", "startup"));
    assert_eq!(out.code, 0, "{}", out.text());
    let agent = smoke.binding(&seat, "codex", "codex-a", "");
    assert_eq!(agent.liveness, "connected");

    // Refused while an agent binding is connected; nothing changes.
    let refused = smoke.human(Some(&pane), &["human", "me", "init"]);
    assert_ne!(refused.code, 0, "{}", refused.text());
    assert_eq!(smoke.bindings(&seat), vec![agent.clone()]);

    // The agent's session ends (a harness end event): its binding is disconnected, and now
    // `me init` succeeds and ends the disconnected agent binding.
    let ended =
        json!({"hook_event_name": "SessionEnd", "session_id": "codex-a", "turn_id": "turn-1"});
    let out = smoke.hook(&codex, &ended);
    assert_eq!(out.code, 0, "{}", out.text());
    assert_eq!(
        smoke.binding(&seat, "codex", "codex-a", "").liveness,
        "disconnected"
    );
    let init = smoke.human(Some(&pane), &["human", "me", "init"]);
    assert_eq!(init.code, 0, "{}", init.text());
    let open = smoke.bindings(&seat);
    assert_eq!(
        open.len(),
        1,
        "a human seat holds only its human binding: {open:?}"
    );
    assert_eq!(open[0].harness, "human", "{open:?}");
    assert!(
        smoke
            .all_bindings(&seat)
            .iter()
            .any(|b| b.id == agent.id && b.ended),
        "me init ended the disconnected agent binding"
    );

    // An agent's check-in on the human seat ends the human binding.
    let claude = Agent::claude("sess-agent", &pane);
    let out = smoke.hook(&claude, &claude_start("sess-agent", "startup"));
    assert_eq!(out.code, 0, "{}", out.text());
    let open = smoke.bindings(&seat);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].harness, "claude", "{open:?}");
    assert!(
        smoke
            .all_bindings(&seat)
            .iter()
            .any(|b| b.harness == "human" && b.ended),
        "the human binding ended"
    );

    // The ended Codex session resumes afterwards: a fresh binding, not the ended row reopened.
    let out = smoke.hook(&codex, &codex_start("codex-a", "resume"));
    assert_eq!(out.code, 0, "{}", out.text());
    let fresh = smoke.binding(&seat, "codex", "codex-a", "");
    assert_ne!(fresh.id, agent.id, "{fresh:?}");
    assert!(
        smoke
            .all_bindings(&seat)
            .iter()
            .any(|b| b.id == agent.id && b.ended),
        "the ended row stays ended"
    );
}

// ---- configuration 5 -----------------------------------------------------------------------

fn codex_end(session: &str) -> Value {
    json!({"hook_event_name": "SessionEnd", "session_id": session, "turn_id": "turn-1"})
}

// Kills: a resume that skips mail it could have shown, one that dumps a backlog past one page
// instead of fast-forwarding, a fast-forward that moves cursors short of the head, and one that
// drops a pending ACK obligation (obligations are never skipped).
#[test]
fn config5_resume_within_a_page_and_past_it() {
    let Some(mut smoke) = Smoke::ticking("config5_resume_within_a_page_and_past_it") else {
        return;
    };
    let (pane, peer_pane) = (smoke.pane(), smoke.pane());
    let (seat, _) = (smoke.seat(&pane), smoke.seat(&peer_pane));
    let codex = Agent::codex("codex-r", &pane).thread("codex-r");
    let peer = Agent::claude("sess-peer", &peer_pane);
    assert_eq!(
        smoke
            .hook(&peer, &claude_start("sess-peer", "startup"))
            .code,
        0
    );
    let out = smoke.hook(&codex, &codex_start("codex-r", "startup"));
    assert_eq!(out.code, 0, "{}", out.text());
    let thread = joined_thread(&smoke, &peer, &codex, &seat);
    let binding = smoke.binding(&seat, "codex", "codex-r", "");

    // Within one page: every message is shown by the resume, nothing is skipped.
    assert_eq!(smoke.hook(&codex, &codex_end("codex-r")).code, 0);
    assert_eq!(
        smoke.binding(&seat, "codex", "codex-r", "").liveness,
        "disconnected"
    );
    // All priorities are presented in full by the resume's own hook.
    for n in 1..=3 {
        smoke
            .cli(
                &peer,
                &[
                    "send",
                    &thread,
                    "--nudge",
                    "--body",
                    &format!("within-page-{n}"),
                ],
            )
            .data();
    }
    // A send's delivery row is published by its attention job: resume only after every job has
    // run, or the page (and the cursor it advances) stops before the last send (ht-0zcn).
    smoke.wait_projected();
    let behind = smoke.cursor(&binding.id, &thread).unwrap();
    let out = smoke.hook(&codex, &codex_start("codex-r", "resume"));
    assert_eq!(out.code, 0, "{}", out.text());
    // Hook exit 0 is fail-open: a page-read or completion failure names itself on stderr.
    assert!(out.stderr.is_empty(), "{}", out.text());
    assert!(!out.stdout.contains("resumed past"), "{}", out.stdout);
    assert_eq!(
        smoke.binding(&seat, "codex", "codex-r", "").liveness,
        "connected"
    );
    assert_eq!(
        smoke.cursor(&binding.id, &thread),
        Some(smoke.count(
            "SELECT MAX(sequence) FROM messages WHERE thread_id=?1 AND kind='ordinary'",
            &[&thread]
        )),
        "complete resume display advances the cursor: {}",
        out.stdout
    );
    assert!(smoke.cursor(&binding.id, &thread).unwrap() > behind);
    for n in 1..=3 {
        assert!(
            out.stdout.contains(&format!("within-page-{n}")),
            "{}",
            out.stdout
        );
    }
    let inbox = smoke.cli(&codex, &["inbox"]).data();
    let bodies: Vec<&str> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["body"].as_str())
        .collect();
    assert_eq!(
        bodies,
        Vec::<&str>::new(),
        "the resume already displayed every body: {inbox}"
    );
    // An explicit text inbox has no duplicate bodies to display.
    let shown = smoke.text(&codex, &["inbox"]);
    assert_eq!(shown.code, 0, "{}", shown.text());
    assert!(!shown.stdout.contains("within-page-"), "{}", shown.stdout);
    // The cursor covers every displayed message (the display ACK of the nudged messages appends
    // its own info event after them).
    assert_eq!(
        smoke.cursor(&binding.id, &thread),
        Some(smoke.count(
            "SELECT MAX(sequence) FROM messages WHERE thread_id=?1 AND kind='ordinary'",
            &[&thread]
        ))
    );

    // Past one page: the resume reports the history gap. A retained receipt
    // obligation is still injected and ACKed when its complete body fits.
    assert_eq!(smoke.hook(&codex, &codex_end("codex-r")).code, 0);
    let big = "x".repeat(9_000);
    for n in 1..=2 {
        // Lazy (the default): no obligation, so the resume skips them past the page.
        smoke
            .cli(
                &peer,
                &["send", &thread, "--body", &format!("big-{n} {big}")],
            )
            .data();
    }
    // An ordinary send right after the end reopens the seat's unavailability episode; until the
    // warning worker publishes the previous episode's clear, the send is refused as transient
    // `store_busy`: retry as a caller would.
    let started = std::time::Instant::now();
    let owed = loop {
        let out = smoke.cli(
            &peer,
            &["send", &thread, "--body", "owed", "--require-ack", &seat],
        );
        if out.code == 0 || started.elapsed() > Duration::from_secs(20) {
            break out.data().as_str().unwrap().to_owned();
        }
        assert!(out.text().contains("store_busy"), "{}", out.text());
        std::thread::sleep(Duration::from_millis(100));
    };
    // The resume page's room depends on its check-in text (ht-0zcn): only a warning the
    // attribution worker has projected rides the check-in as an offered notice, and without that
    // line the untrimmed offer just fits under the hook bound and leaves no room for the owed row,
    // which then (by design) waits behind the continuation line. Resume only after the workers
    // have projected everything, so the check-in carries the clear notice.
    smoke.wait_projected();
    let resume_frontier = smoke.head(&thread);
    let out = smoke.hook(&codex, &codex_start("codex-r", "resume"));
    assert_eq!(out.code, 0, "{}", out.text());
    // Hook exit 0 is fail-open: surface post-flush completion failures before
    // asserting receipt state, rather than reporting a misleading inbox race.
    assert!(out.stderr.is_empty(), "{}", out.text());
    assert!(
        out.stdout.contains("resumed past ") && out.stdout.contains(" messages:"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("herdr-threads summary"),
        "{}",
        out.stdout
    );
    // Reconnection can append unavailability-clear events before the page
    // read. The display must cover at least the entire pre-resume head.
    assert!(smoke.cursor(&binding.id, &thread).unwrap() >= resume_frontier);
    assert!(
        out.stdout.contains("offered notices: "),
        "the projected clear notice rides the check-in: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&format!("message {owed} ")),
        "the owed inbox row was injected (a history preview is not delivery): {}",
        out.stdout
    );
    let inbox = smoke.cli(&codex, &["inbox"]).data();
    assert!(
        !inbox["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["message"] == owed.as_str()),
        "the fully injected obligation was settled: {inbox}"
    );
    let ack_binding: String = smoke.db().query_row(
        "SELECT ack_binding_id FROM receipt_state WHERE message_id=?1 AND seat_id=?2 AND state='acked'",
        [&owed, &seat], |r| r.get(0),
    ).unwrap();
    assert_eq!(
        ack_binding, binding.id,
        "the resumed binding completed the receipt"
    );
}

impl Smoke {
    /// Waits until the daemon's workers have projected every warning and send attention job, so
    /// a hook's check-in and page see each published message and notice (ht-0zcn).
    fn wait_projected(&self) {
        wait_until(
            "the warning and attention projections",
            Duration::from_secs(20),
            || {
                self.count(
                    "SELECT (SELECT count(*) FROM warning_jobs WHERE status IN ('pending','failed')) \
                     + (SELECT count(*) FROM work_jobs \
                        WHERE kind='send_attention' AND status IN ('pending','failed'))",
                    &[],
                ) == 0
            },
        );
    }
}

// ---- configuration 6 -----------------------------------------------------------------------

impl Smoke {
    fn seat_state(&self, seat: &str) -> String {
        self.db()
            .query_row("SELECT state FROM seats WHERE id=?1", [seat], |r| r.get(0))
            .unwrap()
    }
    fn archived(&self, thread: &str) -> bool {
        self.count("SELECT archived FROM threads WHERE id=?1", &[&thread]) != 0
    }
}

// Kills: a pane close that leaves a multi-binding seat's bindings connected or open, retirement
// that blocks on the closed seat's still-listed membership, and archival that waits forever on
// members with no connected binding. The quiet window is shortened to 2 s through the daemon's
// settings file; never a real hour.
#[test]
fn config6_pane_close_retires_and_the_inactive_thread_archives() {
    let Some(mut smoke) = Smoke::build(
        "config6_pane_close_retires_and_the_inactive_thread_archives",
        r#""auto_archive_after_ms":2000,"#,
        Some(SMOKE_TICK),
    ) else {
        return;
    };
    // An anchor workspace keeps the isolated server populated whatever is closed.
    let anchor_pane = smoke.pane();
    smoke.seat(&anchor_pane);
    let anchor = Agent::claude("sess-anchor", &anchor_pane);
    assert_eq!(
        smoke
            .hook(&anchor, &claude_start("sess-anchor", "startup"))
            .code,
        0
    );
    let anchor_thread = smoke
        .cli(&anchor, &["thread", "create", "--topic", "anchor"])
        .data()
        .as_str()
        .unwrap()
        .to_owned();
    let (pane_a, pane_b) = (smoke.pane(), smoke.pane());
    let (seat_a, seat_b) = (smoke.seat(&pane_a), smoke.seat(&pane_b));
    let a1 = Agent::claude("sess-a1", &pane_a);
    let a2 = Agent::claude("sess-a2", &pane_a).in_process(smoke.stand_in());
    let b = Agent::claude("sess-b", &pane_b);
    for who in [&a1, &a2, &b] {
        assert_eq!(
            smoke.hook(who, &claude_start(&who.session, "startup")).code,
            0
        );
    }
    assert_eq!(smoke.bindings(&seat_a).len(), 2);
    let thread = joined_thread(&smoke, &b, &a1, &seat_a);
    smoke.cli(&b, &["send", &thread, "--body", "hello"]).data();

    // Closing pane A retires seat A: both of its bindings disconnect and end, its membership
    // retires, and the seat record stays.
    smoke.herdr_api(&["pane", "close", &pane_a]);
    wait_until("seat A to retire", Duration::from_secs(60), || {
        smoke.seat_state(&seat_a) == "retired"
    });
    for binding in smoke.all_bindings(&seat_a) {
        assert!(binding.ended, "{binding:?}");
        assert_eq!(binding.liveness, "disconnected", "{binding:?}");
        assert_eq!(
            binding.reason.as_deref(),
            Some("seat_retired"),
            "{binding:?}"
        );
    }
    assert_eq!(smoke.all_bindings(&seat_a).len(), 2);
    assert_eq!(
        smoke.count("SELECT COUNT(*) FROM seats WHERE id=?1", &[&seat_a]),
        1
    );
    assert_eq!(smoke.seat_state(&seat_b), "resolved");
    assert!(!smoke.archived(&thread), "seat B is still a live member");

    // Closing pane B too leaves no active participant: the thread archives after the window.
    smoke.herdr_api(&["pane", "close", &pane_b]);
    wait_until("seat B to retire", Duration::from_secs(60), || {
        smoke.seat_state(&seat_b) == "retired"
    });
    wait_until(
        "seat B's membership to retire",
        Duration::from_secs(60),
        || {
            smoke.count(
                "SELECT COUNT(*) FROM memberships WHERE thread_id=?1 AND state='joined'",
                &[&thread],
            ) == 0
        },
    );
    // The archival lane paces itself at one pass a minute, but every commit kicks it (and a
    // pass that runs after the quiet window moves the scan on a phase). The anchor's chatter in
    // its own thread supplies the kicks; it touches nothing in the thread under test.
    let mut chatter = 0;
    wait_until(
        "the inactive thread to archive",
        Duration::from_secs(60),
        || {
            chatter += 1;
            smoke
                .cli(
                    &anchor,
                    &["send", &anchor_thread, "--body", &format!("tick {chatter}")],
                )
                .data();
            smoke.archived(&thread)
        },
    );
}

// ---- configuration 7 -----------------------------------------------------------------------

// Kills: a Herdr restart (a new incarnation) read as pane closes - retiring seats or archiving
// their threads - instead of holding them for restore.
#[test]
fn config7_herdr_restart_keeps_seats_in_restore_hold() {
    let Some(mut smoke) = Smoke::new("config7_herdr_restart_keeps_seats_in_restore_hold") else {
        return;
    };
    let (pane, peer_pane) = (smoke.pane(), smoke.pane());
    let (seat, peer_seat) = (smoke.seat(&pane), smoke.seat(&peer_pane));
    let one = Agent::claude("sess-one", &pane);
    let two = Agent::claude("sess-two", &pane).in_process(smoke.stand_in());
    let peer = Agent::claude("sess-peer", &peer_pane);
    for who in [&one, &two, &peer] {
        assert_eq!(
            smoke.hook(who, &claude_start(&who.session, "startup")).code,
            0
        );
    }
    let thread = joined_thread(&smoke, &peer, &one, &seat);
    let before = smoke.bindings(&seat);
    assert_eq!(before.len(), 2);

    smoke.herdr.restart();
    wait_until(
        "the seats to go unresolved",
        Duration::from_secs(60),
        || smoke.seat_state(&seat) == "unresolved" && smoke.seat_state(&peer_seat) == "unresolved",
    );
    // Held, not retired: bindings ended as unresolved (not retired), memberships kept, nothing
    // archived, no retirement recorded, and the instance holds an unclaimed baseline.
    for binding in smoke.all_bindings(&seat) {
        assert_eq!(
            binding.reason.as_deref(),
            Some("seat_unresolved"),
            "{binding:?}"
        );
        assert_eq!(binding.liveness, "disconnected", "{binding:?}");
    }
    assert_eq!(
        smoke.count(
            "SELECT COUNT(*) FROM memberships WHERE thread_id=?1 AND state='joined'",
            &[&thread]
        ),
        2
    );
    assert_eq!(
        smoke.count("SELECT COUNT(*) FROM retirements", &[]),
        0,
        "no retirement"
    );
    assert_eq!(
        smoke.count("SELECT COUNT(*) FROM seats WHERE state='retired'", &[]),
        0
    );
    assert!(!smoke.archived(&thread), "no archival");
    assert_eq!(
        smoke.count("SELECT baseline_hold_unclaimed FROM herdr_hosts", &[]),
        1,
        "{}",
        smoke.dump("herdr_hosts")
    );
    // While held, nothing re-enrolls on the old pane id (the binding rows stay for a restore):
    // a resume is told the mapping is unresolved and opens no binding, and a new claim of the
    // pane is refused. The resumed session is one the seat never held: a session the seat
    // does hold matches C1 on the held pane and reattaches (ht-54r.78).
    let stranger = Agent::claude("sess-stranger", &pane);
    let out = smoke.hook(&stranger, &claude_start("sess-stranger", "resume"));
    assert_eq!(
        out.code,
        0,
        "a hook never fails its harness: {}",
        out.text()
    );
    assert!(out.stdout.contains("Unresolved"), "{}", out.text());
    assert!(
        smoke.bindings(&seat).is_empty(),
        "{:?}",
        smoke.bindings(&seat)
    );
    assert_eq!(
        smoke.all_bindings(&seat).len(),
        2,
        "the rows are kept, not deleted"
    );
    let claim = smoke.human(None, &["seat", "resolve", "--pane", &pane]);
    assert_ne!(claim.code, 0, "{}", claim.text());
    assert!(
        claim.text().contains("target_unresolved"),
        "{}",
        claim.text()
    );
}

// ---- configuration 9 -----------------------------------------------------------------------

/// The mod's `watch` child of one binding, its stdout lines forwarded by a reader thread.
struct Watch {
    child: spawn::OwnedChild,
    lines: std::sync::mpsc::Receiver<Value>,
    seen: Vec<Value>,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl Watch {
    fn wait_line(&mut self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let remaining = until.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(remaining) {
                Ok(line) => {
                    self.seen.push(line.clone());
                    if pred(&line) {
                        return line;
                    }
                }
                Err(_) => panic!("timed out waiting for {what}; saw {:#?}", self.seen),
            }
        }
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.child.stop();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Smoke {
    /// `herdr-threads watch` as the Claude mod spawns it for `who`'s session.
    fn watch(&self, who: &Agent) -> Watch {
        use std::io::BufRead;
        let argv: Vec<String> = self
            .routing()
            .into_iter()
            .chain(
                [
                    "watch",
                    "--harness",
                    "claude",
                    "--session",
                    who.session.as_str(),
                ]
                .map(String::from),
            )
            .collect();
        let mut command = self.agent_command(who, &argv);
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn_owned()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && tx.send(value).is_err()
                {
                    return;
                }
            }
        });
        let mut watch = Watch {
            child,
            lines,
            seen: Vec::new(),
            reader: Some(reader),
        };
        watch.wait_line("the connected status", |l| {
            l["kind"] == "status" && l["state"] == "connected"
        });
        watch
    }

    /// The intended foreground process, not a transient child of the pane's starting shell.
    fn foreground_pid(&self, pane: &str, expected_argv: &[&str]) -> Option<i64> {
        use herdr_threads::ports::ProcessInfo;
        let out = self
            .herdr
            .command("herdr")
            .args(["pane", "process-info", "--pane", pane])
            .output()
            .ok()?;
        let value: Value = serde_json::from_slice(&out.stdout).ok()?;
        let info = &value["result"]["process_info"];
        let shell = info["shell_pid"].as_i64();
        let mut pids = info["foreground_processes"]
            .as_array()?
            .iter()
            .filter_map(|p| p["pid"].as_i64())
            .filter(|pid| Some(*pid) != shell)
            .filter(|pid| {
                u32::try_from(*pid)
                    .ok()
                    .and_then(|pid| {
                        herdr_threads::host::process_table::HostProcessTable.process(pid)
                    })
                    .is_some_and(|process| {
                        process
                            .argv
                            .iter()
                            .map(String::as_str)
                            .eq(expected_argv.iter().copied())
                    })
            });
        let pid = pids.next()?;
        pids.next().is_none().then_some(pid)
    }
}

// Kills: bindings of one pane sharing a cursor or a channel (a background session reading or
// settling the foreground's mail), a composer-wake target chosen by anything but "fingerprint pid
// is Herdr's foreground process", a background binding with no channel push of its own, and a
// `claude -p` one-shot that takes over (or is counted as) a session.
#[test]
fn config9_foreground_and_background_agents_in_one_pane() {
    let Some(mut smoke) = Smoke::ticking("config9_foreground_and_background_agents_in_one_pane")
    else {
        return;
    };
    let (pane, peer_pane) = (smoke.pane(), smoke.pane());
    let (seat, _) = (smoke.seat(&pane), smoke.seat(&peer_pane));
    // The foreground agent is the process Herdr sees in the pane.
    let run = smoke
        .herdr
        .command("herdr")
        .args(["pane", "run", &pane, "/bin/sleep 86400"])
        .output()
        .unwrap();
    assert!(run.status.success(), "{run:?}");
    let mut foreground_pid = None;
    wait_until(
        "Herdr to report the pane's foreground process",
        Duration::from_secs(20),
        || {
            // A starting shell can briefly have another foreground child. Its pid can die
            // before SessionStart, leaving no harness fingerprint or lifecycle context.
            foreground_pid = smoke.foreground_pid(&pane, &["/bin/sleep", "86400"]);
            foreground_pid.is_some()
        },
    );
    let foreground_pid = foreground_pid.unwrap();
    let fg = Agent::claude("sess-fg", &pane).in_process(foreground_pid);
    let bg = Agent::claude("sess-bg", &pane).in_process(smoke.stand_in());
    let peer = Agent::claude("sess-peer", &peer_pane);
    for who in [&fg, &peer] {
        assert_eq!(
            smoke.hook(who, &claude_start(&who.session, "startup")).code,
            0
        );
    }
    let thread = joined_thread(&smoke, &peer, &fg, &seat);
    // The background session and a `claude -p` one-shot join the pane after the thread exists.
    assert_eq!(smoke.hook(&bg, &claude_start("sess-bg", "startup")).code, 0);
    // A long-lived stand-in, not a shim that exits at once: the liveness worker (5 s) would
    // otherwise disconnect the one-shot before the assertions below read it as connected.
    let one_shot_pid = smoke.stand_in();
    let one_shot = Agent::claude("sess-p", &pane).in_process(one_shot_pid);
    let created = smoke.cli(&one_shot, &["thread", "create", "--topic", "one-shot's"]);
    assert_eq!(created.code, 0, "{}", created.text());

    // Separate bindings, all top level, each with its own fingerprint.
    let (b_fg, b_bg, b_p) = (
        smoke.binding(&seat, "claude", "sess-fg", ""),
        smoke.binding(&seat, "claude", "sess-bg", ""),
        smoke.binding(&seat, "claude", "sess-p", ""),
    );
    let ids = [&b_fg.id, &b_bg.id, &b_p.id];
    assert!(
        ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2],
        "{ids:?}"
    );
    for binding in [&b_fg, &b_bg, &b_p] {
        assert_eq!(binding.role, "top_level", "{binding:?}");
        assert_eq!(binding.liveness, "connected", "{binding:?}");
    }
    assert_eq!(b_fg.pid, Some(foreground_pid));
    assert_ne!(b_bg.pid, Some(foreground_pid));
    assert_ne!(b_p.pid, Some(foreground_pid));

    // Kills: a composer wake that guesses. Asserted through the production predicate
    // (`wake_targets` + `ReservedWakeAuthority::composer_choice`): the foreground binding only,
    // and nothing for a pid no target holds. Herdr's foreground pid is available here.
    let wake_pid = u32::try_from(foreground_pid).unwrap();
    let targets = herdr_threads::store::wake::wake_targets(
        &smoke.db(),
        &herdr_threads::protocol::ids::SeatId::new(seat.as_str()),
        true,
    )
    .unwrap();
    let mut target_ids: Vec<&str> = targets.iter().map(|t| t.binding.as_str()).collect();
    target_ids.sort_unstable();
    let mut expected_ids = vec![b_fg.id.as_str(), b_bg.id.as_str(), b_p.id.as_str()];
    expected_ids.sort_unstable();
    assert_eq!(
        target_ids, expected_ids,
        "the wake targets are the three bindings"
    );
    // One more than the largest target pid is, by construction, no target's pid.
    let absent = targets
        .iter()
        .filter_map(|t| t.fingerprint_pid)
        .max()
        .unwrap()
        + 1;
    let authority = herdr_threads::ports::ReservedWakeAuthority::Cooperative {
        terminal: herdr_threads::protocol::ids::TerminalId::new("t"),
        incarnation: "34750f98bd59fcfc946da45aaabe933b".into(),
        binding_generation: None,
        harness: None,
        targets,
    };
    match authority.composer_choice(&[wake_pid]) {
        herdr_threads::ports::ComposerChoice::Binding(t) => assert_eq!(
            t.binding.as_str(),
            b_fg.id,
            "the foreground binding is the composer target"
        ),
        other => panic!("the foreground binding is the composer target: {other:?}"),
    }
    assert_eq!(
        authority.composer_choice(&[absent]),
        herdr_threads::ports::ComposerChoice::NoMatch,
        "no match, no composer wake"
    );
    assert_eq!(
        authority.composer_choice(&[]),
        herdr_threads::ports::ComposerChoice::NoMatch,
        "several targets, no foreground pid: no composer wake"
    );

    // Each Claude session has its own mod channel; mail pushes to both.
    let mut fg_watch = smoke.watch(&fg);
    let mut bg_watch = smoke.watch(&bg);
    let message = smoke
        .cli(&peer, &["send", &thread, "--body", "for everyone"])
        .data()
        .as_str()
        .unwrap()
        .to_owned();
    let is_item = |m: String| {
        move |l: &Value| {
            l["id"] == m.as_str() && matches!(l["kind"].as_str(), Some("message" | "lazy"))
        }
    };
    fg_watch.wait_line("the foreground's own push", is_item(message.clone()));
    bg_watch.wait_line(
        "the background's push through its channel",
        is_item(message.clone()),
    );

    // Independent cursors: the foreground's display moves only its own.
    let join_point = smoke.cursor(&b_fg.id, &thread).unwrap();
    assert!(join_point < smoke.head(&thread));
    assert_eq!(smoke.text(&fg, &["inbox"]).code, 0);
    assert_eq!(smoke.cursor(&b_fg.id, &thread), Some(smoke.head(&thread)));
    assert!(
        smoke.cursor(&b_bg.id, &thread).unwrap() < smoke.head(&thread),
        "the background binding has not read: independent cursors"
    );
    assert_eq!(smoke.text(&bg, &["inbox"]).code, 0);
    assert_eq!(smoke.cursor(&b_bg.id, &thread), Some(smoke.head(&thread)));

    // The one-shot exits: its binding disconnects, the sessions stay.
    smoke.end_stand_in(one_shot_pid);
    wait_until(
        "the one-shot to disconnect",
        Duration::from_secs(30),
        || smoke.binding(&seat, "claude", "sess-p", "").liveness == "disconnected",
    );
    for session in ["sess-fg", "sess-bg"] {
        assert_eq!(
            smoke.binding(&seat, "claude", session, "").liveness,
            "connected",
            "{session}"
        );
    }
}

impl Smoke {
    /// `herdr-threads watch ack` as the Claude mod runs it for `who`'s session: one JSON line per id.
    fn watch_ack(&self, who: &Agent, via: &str, id: &str) -> Value {
        let argv: Vec<String> = self
            .routing()
            .into_iter()
            .chain(
                [
                    "watch",
                    "ack",
                    "--session",
                    who.session.as_str(),
                    "--via",
                    via,
                    id,
                ]
                .map(String::from),
            )
            .collect();
        let out = Self::finish(self.agent_command(who, &argv), None);
        assert_eq!(out.code, 0, "{}", out.text());
        out.stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|line| line["id"] == id)
            .unwrap_or_else(|| panic!("no ack line for {id}: {}", out.text()))
    }
}

// Kills: a watch drain that reads the seat instead of its binding (a message a sibling binding
// settled never reaches the still-unread binding, on its first drain or after a reconnect), a mod
// that does not ack such a message (the binding's cursor never passes it), an ack that records a
// second settlement, and a seat-union lazy page that re-shows a row the binding completed while a
// sibling still has it unread.
#[test]
fn config9_a_late_drain_after_a_sibling_settles_still_delivers() {
    let Some(mut smoke) = Smoke::new("config9_a_late_drain_after_a_sibling_settles_still_delivers")
    else {
        return;
    };
    let (pane, peer_pane) = (smoke.pane(), smoke.pane());
    let (seat, _) = (smoke.seat(&pane), smoke.seat(&peer_pane));
    let fg = Agent::claude("sess-fg", &pane).in_process(smoke.stand_in());
    let bg = Agent::claude("sess-bg", &pane).in_process(smoke.stand_in());
    let peer = Agent::claude("sess-peer", &peer_pane);
    for who in [&fg, &peer] {
        assert_eq!(
            smoke.hook(who, &claude_start(&who.session, "startup")).code,
            0
        );
    }
    let thread = joined_thread(&smoke, &peer, &fg, &seat);
    assert_eq!(smoke.hook(&bg, &claude_start("sess-bg", "startup")).code, 0);
    let (b_fg, b_bg) = (
        smoke.binding(&seat, "claude", "sess-fg", ""),
        smoke.binding(&seat, "claude", "sess-bg", ""),
    );
    for binding in [&b_fg, &b_bg] {
        assert_eq!(binding.liveness, "connected", "{binding:?}");
    }

    // An ordinary, ACK-required message; the foreground's text inbox settles it for the seat.
    let message = smoke
        .cli(
            &peer,
            &[
                "send",
                &thread,
                "--body",
                "settle me",
                "--require-ack",
                &seat,
            ],
        )
        .data()
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(smoke.text(&fg, &["inbox"]).code, 0);
    let receipt = |smoke: &Smoke| -> (String, Option<i64>, Option<String>) {
        smoke.db()
            .query_row(
                "SELECT state, acked_at, ack_binding_id FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
                rusqlite::params![message, seat],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    };
    let settled = receipt(&smoke);
    assert_eq!(settled.0, "acked", "the foreground settled it for the seat");
    // The settlement appends ack events after the message, so positions compare to its sequence.
    let sequence: i64 = smoke.count("SELECT sequence FROM messages WHERE id=?1", &[&message]);
    assert!(
        smoke.cursor(&b_fg.id, &thread).unwrap() >= sequence,
        "the foreground read it"
    );
    assert!(
        smoke.cursor(&b_bg.id, &thread).unwrap() < sequence,
        "the background binding has not read"
    );

    // The background's channel still shows it, as an already-settled message.
    let shown = |m: String| move |l: &Value| l["id"] == m.as_str() && l["kind"] == "message";
    let mut bg_watch = smoke.watch(&bg);
    let line = bg_watch.wait_line(
        "the already-settled message on the first drain",
        shown(message.clone()),
    );
    assert_eq!(line["ack_required"], false, "{line}");
    // A reconnect before any ack shows it again.
    drop(bg_watch);
    let mut bg_watch = smoke.watch(&bg);
    bg_watch.wait_line(
        "the message again after a reconnect",
        shown(message.clone()),
    );

    // The mod acks it: only the background's read position moves.
    let acked = smoke.watch_ack(&bg, "context", &message);
    assert_eq!(acked["result"], "already_settled", "{acked}");
    assert!(
        smoke.cursor(&b_bg.id, &thread).unwrap() >= sequence,
        "the ack moved the background's own read position"
    );
    assert_eq!(receipt(&smoke), settled, "no second settlement");

    // A lazy row the background completed is not shown to it again while the foreground still
    // has it unread.
    let lazy = smoke
        .cli(&peer, &["send", &thread, "--body", "lazy"])
        .data()
        .as_str()
        .unwrap()
        .to_owned();
    let is_lazy = |m: String| move |l: &Value| l["id"] == m.as_str() && l["kind"] == "lazy";
    bg_watch.wait_line("the lazy row", is_lazy(lazy.clone()));
    let acked = smoke.watch_ack(&bg, "append", &lazy);
    assert_eq!(acked["result"], "settled", "{acked}");
    drop(bg_watch);
    let bg_watch = smoke.watch(&bg);
    // The connected status came after the first drain was requested; give the drain time to
    // answer, then read everything that arrived.
    let until = Instant::now() + Duration::from_secs(3);
    while let Ok(line) = bg_watch
        .lines
        .recv_timeout(until.saturating_duration_since(Instant::now()))
    {
        assert!(
            line["id"] != lazy.as_str(),
            "the completed lazy row is shown again: {line}"
        );
    }
}
