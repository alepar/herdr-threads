//! H1g (ht-kqrt.48): messaging end to end from a herdr-core terminal site against `fake_hc`.
//!
//! One real `herdr-threads daemon run` child (the production host factory: the kind probe, then
//! `HcHost`) over one fake core with the terminals `t2` and `t3`. Each agent stand-in runs the
//! real binary under a synthetic `claude` parent with `HC_TERMINAL_ID` / `HC_TERMINAL_SOCKET` and
//! no Herdr environment: its hook enrolls the terminal's seat, its CLI sends, reads and ACKs, and
//! its turn hook delivers.

use herdr_threads::{
    protocol::capabilities::{HOST_SITES_V1, TERMINAL_SITES_V1},
    test_support::{
        fake_hc::{FakeHc, FakeTerminal},
        isolation::{TestIsolation, as_agent},
        resolve_seeded,
        spawn::{OwnedChild, SpawnOwned},
        spawn_node_daemon,
    },
};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

/// What one stand-in's process printed and how it ended.
struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

struct Rig {
    // Drop order: the daemon stops before the core and the isolation under it go.
    daemon: Option<OwnedChild>,
    fake: FakeHc,
    isolation: TestIsolation,
    state: PathBuf,
}

impl Rig {
    /// A core with the live terminals `terminals`, and a daemon over a fresh state directory that
    /// knows no host yet: the first terminal claim attaches the core.
    fn start(label: &str, terminals: &[&str]) -> Self {
        let isolation = TestIsolation::new(label);
        let fake = FakeHc::start();
        for terminal in terminals {
            fake.add_terminal(FakeTerminal::live(terminal));
        }
        let state = std::fs::canonicalize(isolation.state_root()).expect("the state root exists");
        resolve_seeded(&state);
        let daemon = spawn_node_daemon(
            &isolation,
            BIN,
            &state,
            None,
            &isolation.path("daemon.stderr"),
        )
        .expect("spawn the daemon");
        let rig = Self {
            daemon: Some(daemon),
            fake,
            isolation,
            state,
        };
        rig.wait_for_daemon();
        rig
    }

    fn wait_for_daemon(&self) {
        let until = Instant::now() + Duration::from_secs(30);
        while Instant::now() < until {
            let ran = self.plain(&["daemon", "health"]);
            if ran.code == Some(0) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!(
            "the daemon did not come up: {}",
            std::fs::read_to_string(self.isolation.path("daemon.stderr")).unwrap_or_default()
        );
    }

    fn socket(&self) -> &Path {
        self.fake.socket()
    }

    /// The state directory, then `args`, as a process outside any terminal.
    fn plain(&self, args: &[&str]) -> Ran {
        let mut command = self.isolation.command(BIN);
        command.arg("--state-dir").arg(&self.state).args(args);
        finish(command)
    }

    /// `command` as the agent of `session` in `terminal`: the core environment and nothing of
    /// Herdr, the session's long-lived stand-in harness process as its liveness.
    fn as_terminal_agent(
        &self,
        command: &mut std::process::Command,
        terminal: &str,
        session: &str,
    ) {
        as_agent(command, "claude", session);
        command
            .env("HC_TERMINAL_ID", terminal)
            .env("HC_TERMINAL_SOCKET", self.socket());
    }

    /// `args` as the agent of `session` in terminal `terminal`: the real binary under a synthetic
    /// `claude` parent, with the core environment and nothing of Herdr.
    fn agent(&self, terminal: &str, session: &str, args: &[&str], stdin: Option<&[u8]>) -> Ran {
        let mut command = self.isolation.command(BIN);
        command.arg("--state-dir").arg(&self.state).args(args);
        self.as_terminal_agent(&mut command, terminal, session);
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        run_with_stdin(command, stdin)
    }

    /// The mod's `watch` child of `session` in `terminal`: its stdout lines arrive on the channel.
    fn watch(&self, terminal: &str, session: &str) -> Watch {
        use std::io::BufRead;
        let mut command = self.isolation.command(BIN);
        command.arg("--state-dir").arg(&self.state).args([
            "watch",
            "--harness",
            "claude",
            "--session",
            session,
        ]);
        self.as_terminal_agent(&mut command, terminal, session);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child: OwnedChild = command.spawn_owned().expect("spawn the watch child");
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Watch {
            lines,
            _child: child,
        }
    }

    /// The SessionStart hook of `session` in `terminal`.
    fn session_start(&self, terminal: &str, session: &str) -> Ran {
        let payload = format!(
            r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"startup"}}"#
        );
        self.agent(
            terminal,
            session,
            &["hook", "claude"],
            Some(payload.as_bytes()),
        )
    }

    /// The seat of `terminal`'s hook-enrolled binding.
    fn seat_of(&self, terminal: &str) -> String {
        self.db()
            .query_row(
                "SELECT id FROM seats WHERE target_id=?1 AND state<>'retired'",
                [terminal],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("a seat on {terminal}: {e}"))
    }

    /// The UserPromptSubmit hook of `session` in `terminal`.
    fn prompt(&self, terminal: &str, session: &str) -> Ran {
        let payload = format!(
            r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"UserPromptSubmit","prompt":"go"}}"#
        );
        self.agent(
            terminal,
            session,
            &["hook", "claude"],
            Some(payload.as_bytes()),
        )
    }

    /// `args` as an agent of `session` whose environment names the core at `socket` and
    /// terminal `terminal`, and also the Herdr pane `pane` when given (a two-family caller).
    fn agent_in(
        &self,
        socket: &Path,
        pane: Option<&str>,
        terminal: &str,
        session: &str,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> Ran {
        let mut command = self.isolation.command(BIN);
        command.arg("--state-dir").arg(&self.state).args(args);
        as_agent(&mut command, "claude", session);
        command
            .env("HC_TERMINAL_ID", terminal)
            .env("HC_TERMINAL_SOCKET", socket);
        if let Some(pane) = pane {
            // A Herdr pane's environment also names its server; nothing listens there.
            command
                .env("HERDR_PANE_ID", pane)
                .env("HERDR_ENV", "1")
                .env("HERDR_SOCKET_PATH", self.isolation.path("herdr.sock"))
                // The CLI's two-family guard finds the daemon from the environment, not from
                // `--state-dir`.
                .env("HERDR_PLUGIN_STATE_DIR", &self.state);
        }
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        run_with_stdin(command, stdin)
    }

    /// The SessionStart hook of `session` for a caller as in [`Self::agent_in`].
    fn session_start_in(
        &self,
        socket: &Path,
        pane: Option<&str>,
        terminal: &str,
        session: &str,
    ) -> Ran {
        let payload = format!(
            r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"startup"}}"#
        );
        self.agent_in(
            socket,
            pane,
            terminal,
            session,
            &["hook", "claude"],
            Some(payload.as_bytes()),
        )
    }

    /// `args` as a person (no agent environment) whose environment names the core at `socket`
    /// and terminal `terminal`, and also the Herdr pane `pane` when given.
    fn person_in(&self, socket: &Path, pane: Option<&str>, terminal: &str, args: &[&str]) -> Ran {
        // The `human` namespace comes first: `human --state-dir DIR me init`.
        let mut command = self.isolation.command(BIN);
        let (namespace, rest) = args.split_first().expect("a namespace");
        command
            .arg(namespace)
            .arg("--state-dir")
            .arg(&self.state)
            .args(rest);
        command
            .env("HC_TERMINAL_ID", terminal)
            .env("HC_TERMINAL_SOCKET", socket);
        if let Some(pane) = pane {
            command
                .env("HERDR_PANE_ID", pane)
                .env("HERDR_ENV", "1")
                .env("HERDR_SOCKET_PATH", self.isolation.path("herdr.sock"))
                .env("HERDR_PLUGIN_STATE_DIR", &self.state);
        }
        run_with_stdin(command, None)
    }

    /// [`Self::watch`] for the terminal of the core at `socket`, optionally also naming a pane.
    fn watch_in(&self, socket: &Path, pane: Option<&str>, terminal: &str, session: &str) -> Watch {
        use std::io::BufRead;
        let mut command = self.isolation.command(BIN);
        command.arg("--state-dir").arg(&self.state).args([
            "watch",
            "--harness",
            "claude",
            "--session",
            session,
        ]);
        as_agent(&mut command, "claude", session);
        command
            .env("HC_TERMINAL_ID", terminal)
            .env("HC_TERMINAL_SOCKET", socket);
        if let Some(pane) = pane {
            command
                .env("HERDR_PANE_ID", pane)
                .env("HERDR_ENV", "1")
                .env("HERDR_SOCKET_PATH", self.isolation.path("herdr.sock"))
                .env("HERDR_PLUGIN_STATE_DIR", &self.state);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child: OwnedChild = command.spawn_owned().expect("spawn the watch child");
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Watch {
            lines,
            _child: child,
        }
    }

    /// The seat on `terminal` of the core at `socket`.
    fn seat_on(&self, socket: &Path, terminal: &str) -> String {
        self.db()
            .query_row(
                "SELECT s.id FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id \
                 WHERE s.target_id=?1 AND h.endpoint=?2 AND s.state<>'retired'",
                [terminal, &socket.to_string_lossy()],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("a seat on {terminal} of {}: {e}", socket.display()))
    }

    fn db(&self) -> rusqlite::Connection {
        let paths = resolve_seeded(&self.state);
        let db = rusqlite::Connection::open(paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.plain(&["daemon", "stop"]);
        self.daemon.take();
    }
}

fn finish(command: std::process::Command) -> Ran {
    run_with_stdin(command, None)
}

fn run_with_stdin(mut command: std::process::Command, stdin: Option<&[u8]>) -> Ran {
    use std::io::Write;
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin.is_none() {
        command.stdin(Stdio::null());
    }
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

/// A running `watch` child; stopped with its process group when dropped.
struct Watch {
    lines: std::sync::mpsc::Receiver<String>,
    _child: OwnedChild,
}

impl Watch {
    /// Lines until one satisfies `want`, within 30 s; every line seen is returned for the failure.
    fn until(&self, what: &str, want: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
        let until = Instant::now() + Duration::from_secs(30);
        let mut seen = Vec::new();
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            let Ok(line) = self.lines.recv_timeout(left) else {
                break;
            };
            let value: serde_json::Value =
                serde_json::from_str(&line).unwrap_or_else(|e| panic!("watch line {line:?}: {e}"));
            if want(&value) {
                return value;
            }
            seen.push(line);
        }
        panic!("no watch line for {what}; saw {seen:?}");
    }
}

/// How many `wake=no_wake` warnings a send printed.
fn no_wake_warnings(ran: &Ran) -> usize {
    ran.stderr.matches("wake=no_wake").count() + ran.stdout.matches("wake=no_wake").count()
}

fn context_of(hook: &Ran) -> String {
    let value: serde_json::Value =
        serde_json::from_str(&hook.stdout).unwrap_or_else(|e| panic!("hook stdout: {e}: {hook:?}"));
    value["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_else(|| panic!("hook context in {}", hook.stdout))
        .to_owned()
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

/// The message id a `send` printed (`thread T message M ...`).
fn sent_message(ran: &Ran) -> String {
    let words: Vec<&str> = ran.stdout.split_whitespace().collect();
    let at = words
        .iter()
        .position(|w| *w == "message")
        .expect("a message id");
    words[at + 1].to_owned()
}

// Kills: a terminal caller that gets no seat from its hook, a send from a Terminal site that is
// refused or never reaches the recipient's turn hook, a hook delivery that omits the body, an ACK
// that is lost, and a `wake=no_wake` warning that repeats or never shows (hc has no composer wake).
#[test]
fn t2_seat_send_receive_ack_hook_delivery() {
    let rig = Rig::start("hcm-flow", &["t2", "t3"]);
    for (terminal, session) in [("t2", "s-t2"), ("t3", "s-t3")] {
        let hook = rig.session_start(terminal, session);
        assert_eq!(hook.code, Some(0), "{hook:?}");
        assert!(
            context_of(&hook).contains("Your seat in this pane"),
            "{hook:?}"
        );
    }
    let (t2, t3) = (rig.seat_of("t2"), rig.seat_of("t3"));
    assert_ne!(t2, t3);

    let first = rig.agent(
        "t3",
        "s-t3",
        &[
            "send",
            "--seat",
            &t2,
            "--body",
            "hello t2",
            "--require-ack",
            &t2,
        ],
        None,
    );
    assert_eq!(first.code, Some(0), "{first:?}");
    assert_eq!(no_wake_warnings(&first), 1, "{first:?}");
    assert!(
        first
            .stderr
            .contains(&format!("warning: {t2} wake=no_wake")),
        "{first:?}"
    );
    let message = sent_message(&first);
    let second = rig.agent(
        "t3",
        "s-t3",
        &["send", "--seat", &t2, "--body", "again"],
        None,
    );
    assert_eq!(second.code, Some(0), "{second:?}");
    assert_eq!(
        no_wake_warnings(&second),
        0,
        "once per sender and seat: {second:?}"
    );

    let owed = rig.agent("t2", "s-t2", &["pending-receipts", "--machine"], None);
    assert!(
        owed.stdout.contains(&message),
        "the ACK is owed before the turn: {owed:?}"
    );
    // The recipient's next turn delivers both bodies through its hook, ACKing on display.
    let turn = rig.prompt("t2", "s-t2");
    assert_eq!(turn.code, Some(0), "{turn:?}");
    let context = context_of(&turn);
    assert!(
        context.contains("hello t2") && context.contains("again"),
        "{context}"
    );
    assert!(context.contains(&message), "{context}");
    let owed = rig.agent("t2", "s-t2", &["pending-receipts", "--machine"], None);
    assert!(
        !owed.stdout.contains(&message),
        "the display ACKed the requested receipt: {owed:?}"
    );

    // An explicit ACK by id from the terminal site.
    let third = rig.agent(
        "t3",
        "s-t3",
        &[
            "send",
            "--seat",
            &t2,
            "--body",
            "third",
            "--require-ack",
            &t2,
        ],
        None,
    );
    let id = sent_message(&third);
    let pending = rig.agent("t2", "s-t2", &["pending-receipts", "--machine"], None);
    assert!(pending.stdout.contains(&id), "{pending:?}");
    let ack = rig.agent("t2", "s-t2", &["ack", &id], None);
    assert_eq!(ack.code, Some(0), "{ack:?}");
    let after = rig.agent("t2", "s-t2", &["pending-receipts", "--machine"], None);
    assert!(!after.stdout.contains(&id), "{after:?}");
}

// Kills: a Terminal site that cannot join an ordinary thread or accept an invitation, and an
// `accept` that omits the joined thread's backlog (an extra call would be needed).
#[test]
fn t2_join_accept_prints_backlog() {
    let rig = Rig::start("hcm-accept", &["t2", "t3"]);
    for (terminal, session) in [("t2", "s-t2"), ("t3", "s-t3")] {
        assert_eq!(rig.session_start(terminal, session).code, Some(0));
    }
    let (t2, t3) = (rig.seat_of("t2"), rig.seat_of("t3"));
    let created = rig.agent(
        "t3",
        "s-t3",
        &[
            "thread", "create", "--name", "ops", "--topic", "ops", "--json",
        ],
        None,
    );
    assert_eq!(created.code, Some(0), "{created:?}");
    let invited = rig.agent("t3", "s-t3", &["invite", "ops", "--seat", &t2], None);
    assert_eq!(invited.code, Some(0), "{invited:?}");
    let first = rig.agent("t2", "s-t2", &["accept", "ops"], None);
    assert_eq!(first.code, Some(0), "{first:?}");
    // A backlog t2 never displayed, left across its leave and a new invitation.
    let sent = rig.agent(
        "t3",
        "s-t3",
        &["send", "ops", "--body", "backlog body"],
        None,
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    assert_eq!(
        rig.agent("t2", "s-t2", &["leave", "ops"], None).code,
        Some(0)
    );
    let again = rig.agent("t3", "s-t3", &["invite", "ops", "--seat", &t2], None);
    assert_eq!(again.code, Some(0), "{again:?}");

    let accepted = rig.agent("t2", "s-t2", &["accept", "ops"], None);
    assert_eq!(accepted.code, Some(0), "{accepted:?}");
    assert!(
        accepted.stdout.contains("backlog body"),
        "the accept shows the pending delivery rows itself: {accepted:?}"
    );

    // A second thread joined without an invitation.
    let open = rig.agent(
        "t3",
        "s-t3",
        &["thread", "create", "--name", "open", "--topic", "open"],
        None,
    );
    assert_eq!(open.code, Some(0), "{open:?}");
    let joined = rig.agent("t2", "s-t2", &["join", "open"], None);
    assert_eq!(joined.code, Some(0), "{joined:?}");
    let members = rig.agent("t2", "s-t2", &["participants", "open", "--machine"], None);
    assert!(
        members.stdout.contains(&t2) && members.stdout.contains(&t3),
        "{members:?}"
    );
}

// Kills: a Terminal site whose watch stops on `no_pane`, a channel that does not deliver, and a
// `wake=no_wake` warning for a seat whose live mod channel wakes it.
#[test]
fn live_mod_claude_no_no_wake_hooks_only_once() {
    let rig = Rig::start("hcm-mod", &["t2", "t3"]);
    for (terminal, session) in [("t2", "s-t2"), ("t3", "s-t3")] {
        assert_eq!(rig.session_start(terminal, session).code, Some(0));
    }
    let (t2, t3) = (rig.seat_of("t2"), rig.seat_of("t3"));
    let watch = rig.watch("t2", "s-t2");
    let status = watch.until("connected", |v| {
        v["kind"] == "status" && v["state"] == "connected"
    });
    assert_ne!(status["reason"], "no_pane");

    let live = rig.agent(
        "t3",
        "s-t3",
        &[
            "send",
            "--seat",
            &t2,
            "--body",
            "via channel",
            "--require-ack",
            &t2,
        ],
        None,
    );
    assert_eq!(live.code, Some(0), "{live:?}");
    assert_eq!(
        no_wake_warnings(&live),
        0,
        "a live mod channel wakes it: {live:?}"
    );
    let delivered = watch.until("the message", |v| v["body"] == "via channel");
    assert!(
        matches!(delivered["kind"].as_str(), Some("lazy" | "message")),
        "{delivered}"
    );

    // The hooks-only seat (t3 has no channel) reports once.
    let hooks_only = rig.agent(
        "t2",
        "s-t2",
        &[
            "send",
            "--seat",
            &t3,
            "--body",
            "to hooks",
            "--require-ack",
            &t3,
        ],
        None,
    );
    assert_eq!(no_wake_warnings(&hooks_only), 1, "{hooks_only:?}");
    let again = rig.agent(
        "t2",
        "s-t2",
        &[
            "send",
            "--seat",
            &t3,
            "--body",
            "to hooks 2",
            "--require-ack",
            &t3,
        ],
        None,
    );
    assert_eq!(no_wake_warnings(&again), 0, "{again:?}");
}

// Kills: `seat retire --self` from a Terminal site that retires an unnamed hc seat or prints
// another refusal (golden: tests/cli/golden/m20.txt).
#[test]
fn self_retire_from_terminal_m20() {
    let rig = Rig::start("hcm-retire", &["t2"]);
    assert_eq!(rig.session_start("t2", "s-t2").code, Some(0));
    let t2 = rig.seat_of("t2");
    let ran = rig.agent("t2", "s-t2", &["seat", "retire", "--self"], None);
    assert_ne!(ran.code, Some(0), "{ran:?}");
    assert_eq!(ran.stderr, include_str!("cli/golden/m20.txt"), "{ran:?}");
    let state: String = rig
        .db()
        .query_row("SELECT state FROM seats WHERE id=?1", [&t2], |r| r.get(0))
        .unwrap();
    assert_ne!(state, "retired");
}

// Kills: a verified two-family caller (pane and terminal in its environment, the terminal an
// ancestor) that binds nothing, one that acts as the pane instead, and a hook that enrolls a
// seat on the pane.
#[test]
fn two_family_terminal_acts_as_its_terminal_seat() {
    let rig = Rig::start("hcm-two-family", &["t1", "t2"]);
    // The CLI children are descendants of this test process: it is the terminal's root.
    rig.fake.set_pid("t1", Some(std::process::id()));
    let socket = rig.socket().to_owned();
    let hook = rig.session_start_in(&socket, Some("w9:p9"), "t1", "s-t1");
    assert_eq!(hook.code, Some(0), "{hook:?}");
    assert!(
        context_of(&hook).contains("Your seat in this pane"),
        "{hook:?}"
    );
    let t1 = rig.seat_on(&socket, "t1");
    let panes: i64 = rig
        .db()
        .query_row(
            "SELECT COUNT(*) FROM seats WHERE target_id='w9:p9'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(panes, 0, "no seat names the pane");

    assert_eq!(rig.session_start("t2", "s-t2").code, Some(0));
    let t2 = rig.seat_of("t2");
    let sent = rig.agent_in(
        &socket,
        Some("w9:p9"),
        "t1",
        "s-t1",
        &[
            "send",
            "--seat",
            &t2,
            "--body",
            "from the verified terminal",
        ],
        None,
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    assert!(!sent.stderr.contains("neither is an ancestor"), "{sent:?}");
    let turn = rig.prompt("t2", "s-t2");
    assert!(
        context_of(&turn).contains("from the verified terminal"),
        "{turn:?}"
    );
    // The reply lands in the two-family caller's own seat, read back without naming it.
    let back = rig.agent(
        "t2",
        "s-t2",
        &["send", "--seat", &t1, "--body", "pong"],
        None,
    );
    assert_eq!(back.code, Some(0), "{back:?}");
    let inbox = rig.agent_in(&socket, Some("w9:p9"), "t1", "s-t1", &["inbox"], None);
    assert_eq!(inbox.code, Some(0), "{inbox:?}");
    assert!(inbox.stdout.contains("pong"), "{inbox:?}");
}

// Kills: a lone terminal's seat lookup that sends a bare `tN` (two cores that each seat `t1` fail
// with "maps to more than one seat", or the lookup answers the other core's seat).
#[test]
fn two_cores_sharing_t1_resolve_each_callers_own_seat() {
    let rig = Rig::start("hcm-two-cores", &["t1", "t2"]);
    let other = FakeHc::start();
    other.add_terminal(FakeTerminal::live("t1"));
    other.add_terminal(FakeTerminal::live("t3"));
    let (a, b) = (rig.socket().to_owned(), other.socket().to_owned());
    assert_eq!(rig.session_start_in(&a, None, "t1", "s-a1").code, Some(0));
    assert_eq!(rig.session_start_in(&b, None, "t1", "s-b1").code, Some(0));
    assert_eq!(rig.session_start_in(&a, None, "t2", "s-a2").code, Some(0));
    assert_eq!(rig.session_start_in(&b, None, "t3", "s-b3").code, Some(0));
    let (a1, b1) = (rig.seat_on(&a, "t1"), rig.seat_on(&b, "t1"));
    assert_ne!(a1, b1);
    let (a2, b3) = (rig.seat_on(&a, "t2"), rig.seat_on(&b, "t3"));
    for (socket, session, to, body) in [(&a, "s-a2", &a1, "for a1"), (&b, "s-b3", &b1, "for b1")] {
        let terminal = if socket == &a { "t2" } else { "t3" };
        let sent = rig.agent_in(
            socket,
            None,
            terminal,
            session,
            &["send", "--seat", to, "--body", body],
            None,
        );
        assert_eq!(sent.code, Some(0), "{sent:?}");
    }
    let _ = (a2, b3);
    for (socket, session, want, not) in [
        (&a, "s-a1", "for a1", "for b1"),
        (&b, "s-b1", "for b1", "for a1"),
    ] {
        let inbox = rig.agent_in(socket, None, "t1", session, &["inbox"], None);
        assert_eq!(inbox.code, Some(0), "{inbox:?}");
        assert!(inbox.stdout.contains(want), "{inbox:?}");
        assert!(!inbox.stdout.contains(not), "{inbox:?}");
    }
}

// Kills: an allocate slot that drops the core, so `<session>:t2` allocates on the caller's own host
// or, under a named caller, mints a named seat `t2`.
#[test]
fn allocate_slot_allocates_on_the_named_core() {
    let rig = Rig::start("hcm-allocate", &["t2", "t3"]);
    assert_eq!(rig.session_start("t3", "s-t3").code, Some(0));
    rig.fake.set_core_name("alpha");
    let session = "alpha";
    // A named seat is verified by a real harness-shaped ancestor: one script enrolls it, creates
    // a thread and invites the core's `t2` by its qualified address.
    let script = rig.isolation.path("named-invite.sh");
    std::fs::write(
        &script,
        r#"#!/bin/bash
payload=${HT_START//SESSION/$CLAUDE_CODE_SESSION_ID}
for ((i=0;i<120;i++)); do
  out=$(printf '%s' "$payload" | "$HT_BIN" --state-dir "$HT_STATE" hook claude 2>&1)
  case "$out" in *"Your seat"*) break;; esac
  /bin/sleep 0.5
done
"$HT_BIN" --state-dir "$HT_STATE" thread create --name ops --topic ops || exit 21
"$HT_BIN" --state-dir "$HT_STATE" invite ops --seat "$HT_ADDRESS" || exit 22
"#,
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut command =
        herdr_threads::test_support::harness_shim::under_harness("claude", &script, &[], &[]);
    herdr_threads::test_support::isolation::scrub_env(&mut command);
    command
        .env("HOME", rig.isolation.home())
        .env("HT_BIN", BIN)
        .env("HT_STATE", &rig.state)
        .env(
            "HT_START",
            r#"{"session_id":"SESSION","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"startup"}"#,
        )
        .env("HT_ADDRESS", format!("{session}:t2"))
        .env("HERDR_THREADS_SEAT_NAME", "bob")
        .env("CLAUDE_CODE_SESSION_ID", "s-bob");
    let invited = run_with_stdin(command, None);
    assert_eq!(invited.code, Some(0), "{invited:?}");
    let t2 = rig.seat_of("t2");
    let on_core: i64 = rig
        .db()
        .query_row(
            "SELECT COUNT(*) FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id \
             WHERE s.id=?1 AND h.endpoint=?2",
            [&t2, &rig.socket().to_string_lossy().into_owned()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(on_core, 1, "the seat is on the core's host");
    let named: i64 = rig
        .db()
        .query_row("SELECT COUNT(*) FROM seats WHERE name='t2'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(named, 0, "no named seat t2");
}

/// The core's own methods; any other request on its socket is a Herdr one.
const HC_METHODS: &[&str] = &[
    "hello",
    "server.status",
    "terminal.get",
    "doc.get",
    "events.subscribe",
    "terminal.run",
];

fn assert_core_never_dialled_as_herdr(rig: &Rig) {
    let herdr: Vec<String> = rig
        .fake
        .calls()
        .into_iter()
        .map(|call| call.method)
        .filter(|method| !HC_METHODS.contains(&method.as_str()))
        .collect();
    assert!(
        herdr.is_empty(),
        "Herdr methods sent to the core: {herdr:?}"
    );
}

// Kills: a Terminal site's pane form dialling its core socket as a Herdr server, or sending
// ResolveSeat for a Herdr pane at the core, instead of refusing with M21.
#[test]
fn terminal_site_pane_form_never_dials_the_core() {
    let rig = Rig::start("hcm-pane-form", &["t1"]);
    assert_eq!(rig.session_start("t1", "s-t1").code, Some(0));
    let ran = rig.agent("t1", "s-t1", &["seat", "resolve", "--pane", "w1:p3"], None);
    assert_ne!(ran.code, Some(0), "{ran:?}");
    assert!(
        ran.stderr
            .contains("pane w1:p3 has no reachable Herdr host; pass --host-endpoint"),
        "{ran:?}"
    );
    assert_core_never_dialled_as_herdr(&rig);
}

// Kills: `human me init` from a Terminal site reading the pane agent through its core socket
// as a Herdr server.
#[test]
fn terminal_me_init_never_dials_the_core() {
    let rig = Rig::start("hcm-me-init", &["t1"]);
    let mut command = rig.isolation.command(BIN);
    command
        .arg("human")
        .arg("--state-dir")
        .arg(&rig.state)
        .args(["me", "init"])
        .env("HC_TERMINAL_ID", "t1")
        .env("HC_TERMINAL_SOCKET", rig.socket());
    let ran = finish(command);
    assert_eq!(ran.code, Some(0), "{ran:?}");
    assert_core_never_dialled_as_herdr(&rig);
}

/// A daemon that advertises only the Named-site capability and records what it is sent.
struct NamedOnlyDaemon {
    sent: std::sync::Mutex<usize>,
}

impl herdr_threads::ports::LocalClient for NamedOnlyDaemon {
    fn supports_capability(
        &self,
        name: &str,
        _: &herdr_threads::protocol::time::CallBudget,
    ) -> bool {
        name == HOST_SITES_V1
    }
    fn call(
        &self,
        _: herdr_threads::protocol::commands::Command,
        _: &herdr_threads::protocol::time::CallBudget,
    ) -> Result<
        herdr_threads::protocol::results::CommandResult,
        herdr_threads::protocol::results::ApiError,
    > {
        *self.sent.lock().unwrap() += 1;
        Err(herdr_threads::protocol::results::ApiError::unsupported(
            "not served",
        ))
    }
    fn call_with_output(
        &self,
        command: herdr_threads::protocol::commands::Command,
        _: &herdr_threads::protocol::output::OutputSpec,
        budget: &herdr_threads::protocol::time::CallBudget,
    ) -> Result<
        herdr_threads::protocol::results::CommandResult,
        herdr_threads::protocol::results::ApiError,
    > {
        self.call(command, budget)
    }
}

// Kills: a CLI that sends a Terminal site (a socket host the daemon cannot place) to a daemon
// that only knows Named sites, instead of printing M13 and sending nothing.
#[test]
fn terminal_site_against_host_sites_only_daemon_prints_m13() {
    use herdr_threads::cli::{
        caller_location::CallerLocator,
        site_capability::{M13, resolve_thread_query, seats_host},
    };
    use herdr_threads::protocol::time::{CallBudget, Cancellation, MonoInstant};
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    let daemon = NamedOnlyDaemon {
        sent: std::sync::Mutex::new(0),
    };
    let terminal = CallerLocator::Terminal {
        socket: "/core/work.sock".into(),
        id: "t2".into(),
    };
    let lookup = seats_host(Some(terminal.site()), &daemon, &budget).unwrap_err();
    assert_eq!(lookup.detail, M13);
    let resolve = resolve_thread_query(Some(terminal.site()), "ops", None, None, &daemon, &budget)
        .unwrap_err();
    assert_eq!(resolve.detail, M13);
    assert_eq!(*daemon.sent.lock().unwrap(), 0, "no Terminal site was sent");
    assert_eq!(
        include_str!("cli/golden/m13.txt"),
        format!("{M13}\n"),
        "the line is the Named-site M13"
    );
    // A Named site passes the same daemon.
    let named = CallerLocator::Named(herdr_threads::protocol::ids::SeatName::parse("w").unwrap());
    assert!(
        seats_host(Some(named.site()), &daemon, &budget)
            .unwrap()
            .is_some()
    );
    let _ = TERMINAL_SITES_V1;
}

/// Two cores that each seat `t1`: the first core's `t1` has no seat yet and the second core's
/// has an agent's.
fn two_cores_with_a_seated_other_t1(label: &str) -> (Rig, FakeHc, PathBuf, PathBuf, String) {
    let rig = Rig::start(label, &["t1", "t2"]);
    let other = FakeHc::start();
    other.add_terminal(FakeTerminal::live("t1"));
    let (a, b) = (rig.socket().to_owned(), other.socket().to_owned());
    assert_eq!(rig.session_start_in(&b, None, "t1", "s-b1").code, Some(0));
    let b1 = rig.seat_on(&b, "t1");
    (rig, other, a, b, b1)
}

fn assert_me_init_bound_its_own_core(rig: &Rig, a: &Path, b1: &str, ran: &Ran) {
    assert_eq!(ran.code, Some(0), "{ran:?}");
    assert!(!ran.stderr.contains("more than one seat"), "{ran:?}");
    let a1 = rig.seat_on(a, "t1");
    assert_ne!(
        a1, b1,
        "the person seat is this core's, not the other core's agent"
    );
    let host_of_b1: String = rig
        .db()
        .query_row(
            "SELECT h.endpoint FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id WHERE s.id=?1",
            [b1],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(host_of_b1, a.to_string_lossy());
}

// Kills: `human me init` looking its seat up by a bare `tN` (the other core's `t1` seat is bound
// instead, or two cores seating `t1` fail with "maps to more than one seat").
#[test]
fn two_cores_me_init_resolves_each_callers_own_seat() {
    let (rig, _other, a, _b, b1) = two_cores_with_a_seated_other_t1("hcm-me-init");
    let ran = rig.person_in(&a, None, "t1", &["human", "me", "init"]);
    assert_me_init_bound_its_own_core(&rig, &a, &b1, &ran);
}

// Kills: the same lookup for a two-family caller whose terminal verifies: the guard's swapped env
// must reach `me init` as the (core socket, tN) site.
#[test]
fn two_family_verified_terminal_me_init_uses_the_core() {
    let (rig, other, a, _b, b1) = two_cores_with_a_seated_other_t1("hcm-me-init-2f");
    rig.fake.set_pid("t1", Some(std::process::id()));
    let _ = &other;
    let ran = rig.person_in(&a, Some("w9:p9"), "t1", &["human", "me", "init"]);
    assert_me_init_bound_its_own_core(&rig, &a, &b1, &ran);
}

// Kills: a watch of a verified terminal that registers its channel on the other core's `t1` seat
// (or on the Herdr pane the environment also names).
#[test]
fn two_cores_watch_continuity_stays_on_its_core() {
    let rig = Rig::start("hcm-watch", &["t1", "t2"]);
    let other = FakeHc::start();
    other.add_terminal(FakeTerminal::live("t1"));
    let (a, b) = (rig.socket().to_owned(), other.socket().to_owned());
    rig.fake.set_pid("t1", Some(std::process::id()));
    assert_eq!(rig.session_start_in(&a, None, "t1", "s-a1").code, Some(0));
    assert_eq!(rig.session_start_in(&b, None, "t1", "s-b1").code, Some(0));
    assert_eq!(rig.session_start_in(&a, None, "t2", "s-a2").code, Some(0));
    let (a1, b1) = (rig.seat_on(&a, "t1"), rig.seat_on(&b, "t1"));
    assert_ne!(a1, b1);
    let watch_a = rig.watch_in(&a, Some("w9:p9"), "t1", "s-a1");
    let watch_b = rig.watch_in(&b, None, "t1", "s-b1");
    for watch in [&watch_a, &watch_b] {
        let status = watch.until("connected", |v| {
            v["kind"] == "status" && v["state"] == "connected"
        });
        assert_ne!(status["reason"], "no_pane");
    }
    for (to, body) in [(&a1, "for a1"), (&b1, "for b1")] {
        let sent = rig.agent_in(
            &a,
            None,
            "t2",
            "s-a2",
            &["send", "--seat", to, "--body", body],
            None,
        );
        assert_eq!(sent.code, Some(0), "{sent:?}");
    }
    let seen_a = watch_a.until("a1's message", |v| v["body"].is_string());
    assert_eq!(seen_a["body"], "for a1", "{seen_a}");
    let seen_b = watch_b.until("b1's message", |v| v["body"].is_string());
    assert_eq!(seen_b["body"], "for b1", "{seen_b}");
}
