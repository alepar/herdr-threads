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
