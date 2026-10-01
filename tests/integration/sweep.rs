//! End-to-end main flow through the built `herdr-threads` executable and the
//! detached daemon it ensures. The Herdr host is a private protocol endpoint
//! served by this test process (`ping`, `session.snapshot`, `pane.get`); every
//! other host call is answered with a typed error, so no prompt is ever
//! delivered and nothing here can stand in for a model receipt.

use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::DirBuilderExt, net::UnixListener},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

/// Private scratch root under /private/tmp (short enough for socket paths).
pub(crate) struct Scratch(pub(crate) PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn pane(id: &str, terminal: &str) -> Value {
    // A plain shell pane: no recognized harness agent, so a safe wake is
    // always refused before submission.
    json!({"pane_id":id,"terminal_id":terminal,"workspace_id":"w1","tab_id":"w1:t1",
           "focused":false,"agent_status":"idle","revision":3})
}

/// A pane whose Herdr agent record names `kind` and, when given, an
/// integration `agent_session` value (the stand-in answers `agent.get` from it).
pub(crate) fn agent_pane(id: &str, terminal: &str, kind: &str, session: Option<&str>) -> Value {
    let mut pane = pane(id, terminal);
    pane["agent"] = json!(kind);
    if let Some(value) = session {
        pane["agent_session"] = json!({"agent":kind,"kind":"id",
            "source":format!("herdr:{kind}"),"value":value});
    }
    pane
}

/// The private Herdr endpoint. One request per accepted connection.
pub(crate) struct FakeHost {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl FakeHost {
    pub(crate) fn start(socket: &Path, panes: Vec<Value>) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("fake host accept: {error}"),
                };
                // macOS accept(2) inherits O_NONBLOCK from the listener.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut line = String::new();
                if BufReader::new(&mut stream).read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }
                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let id = request["id"].clone();
                let reply = match request["method"].as_str().unwrap_or_default() {
                    "ping" => {
                        json!({"id":id,"result":{"type":"pong","version":"0.9.1","protocol":22}})
                    }
                    "session.snapshot" => json!({"id":id,"result":{"type":"session_snapshot",
                        "snapshot":{"version":"0.9.1","protocol":22,"panes":panes,
                                    "agents":[],"tabs":[],"workspaces":[],"layouts":[]}}}),
                    "pane.get" => match panes
                        .iter()
                        .find(|pane| pane["pane_id"] == request["params"]["pane_id"])
                    {
                        Some(pane) => json!({"id":id,"result":{"type":"pane_info","pane":pane}}),
                        None => {
                            json!({"id":id,"error":{"code":"pane_not_found","message":"pane not found"}})
                        }
                    },
                    "agent.get" => {
                        let pane = panes
                            .iter()
                            .find(|pane| pane["pane_id"] == request["params"]["target"]);
                        match pane {
                            Some(pane) if pane.get("agent_get_error").is_some() => {
                                json!({"id":id,"error":{"code":pane["agent_get_error"],
                                    "message":"scripted read error"}})
                            }
                            Some(pane) if pane["agent"].is_string() => {
                                let mut agent = json!({"agent":pane["agent"],
                                    "agent_status":pane["agent_status"],
                                    "pane_id":pane["pane_id"],
                                    "terminal_id":pane["terminal_id"]});
                                if let Some(session) = pane.get("agent_session") {
                                    agent["agent_session"] = session.clone();
                                }
                                json!({"id":id,"result":{"type":"agent_info","agent":agent}})
                            }
                            _ => {
                                json!({"id":id,"error":{"code":"agent_not_found","message":"no agent in pane"}})
                            }
                        }
                    }
                    _ => {
                        json!({"id":id,"error":{"code":"agent_not_found","message":"no agent in pane"}})
                    }
                };
                let _ = writeln!(stream, "{reply}");
            }
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for FakeHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// A stand-in occupant: the cooperative caller flags for one seat and pane.
#[derive(Clone, Copy)]
struct Caller<'a> {
    seat: &'a str,
    pane: &'a str,
    role: &'a str,
}

struct Plugin {
    state: PathBuf,
    host: PathBuf,
}
impl Plugin {
    fn command(&self, caller: Option<Caller>, args: &[&str]) -> (i32, Value, String) {
        let mut command = Command::new(BIN);
        command
            .arg("--json")
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.host);
        if let Some(caller) = caller {
            command.args([
                "--cooperative-seat",
                caller.seat,
                "--cooperative-target",
                caller.pane,
                "--cooperative-harness",
                "claude",
                "--cooperative-role",
                caller.role,
            ]);
        }
        let output = command
            .args(args)
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("HERDR_ENV")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let value = serde_json::from_str(&stdout).unwrap_or(Value::Null);
        (output.status.code().unwrap_or(-1), value, stderr)
    }
    /// Success, returning `result.data`.
    fn ok(&self, caller: Option<Caller>, args: &[&str]) -> Value {
        let (code, value, stderr) = self.command(caller, args);
        assert_eq!(code, 0, "herdr-threads {args:?} failed: {stderr}{value}");
        value["result"]["data"].clone()
    }
    fn kind(&self, caller: Option<Caller>, args: &[&str]) -> String {
        let (code, value, stderr) = self.command(caller, args);
        assert_eq!(code, 0, "herdr-threads {args:?} failed: {stderr}{value}");
        value["result"]["kind"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
    fn pending(&self, seat: &str) -> Vec<Value> {
        self.ok(
            None,
            &["pending-receipts", "--seat", seat, "--limit", "100"],
        )["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    fn pending_for(&self, seat: &str, message: &str) -> Option<Value> {
        self.pending(seat)
            .into_iter()
            .find(|item| item.to_string().contains(message))
    }
    fn warnings(&self, seat: &str) -> Vec<Value> {
        self.ok(None, &["warnings", "--seat", seat, "--limit", "100"])["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    fn ensure(&self) -> Value {
        self.ok(None, &["daemon", "ensure"])
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.command(None, &["daemon", "stop"]);
    }
}

fn utc_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn wait_until(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Subjects of the operator overdue listing.
fn overdue_subjects(plugin: &Plugin) -> Vec<String> {
    plugin.ok(None, &["overdue", "--limit", "100"])["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["subject"].as_str().unwrap().to_owned())
        .collect()
}

/// The effective membership state of `seat` in `thread`'s participant page.
fn membership(plugin: &Plugin, caller: Caller, thread: &str, seat: &str) -> String {
    plugin.ok(Some(caller), &["thread", "show", thread])["participants"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["seat"] == seat)
        .map(|item| item["effective_state"].as_str().unwrap().to_owned())
        .unwrap_or_default()
}

/// R5/R8/R9/R10/R11/R14/R16/R17 through the installed executable: a prelaunch
/// invitation plus required-ACK assignment survive a daemon restart before
/// the recipient exists, are discovered at its first check-in, read without
/// settling anything, rejected from a child context, explicitly ACKed and
/// separately accepted by the top-level context, with a started deadline
/// preserved across a second restart. Then a missed deadline yields exactly
/// one durable warning while a timely ACK stays quiet, leave keeps earlier
/// obligations and stops future fanout, a left seat is reinvited in a new
/// episode, and archive refuses discussion while permitting a late ACK and a
/// late acceptance.
#[test]
fn installed_flow_prelaunch_handoff_to_explicit_receipt_survives_daemon_restart() {
    let root = PathBuf::from(format!(
        "/private/tmp/htsweep-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let _scratch = Scratch(root.clone());
    let socket = root.join("herdr.sock");
    let _host = FakeHost::start(
        &socket,
        vec![pane("w1:p1", "term-a"), pane("w1:p2", "term-b")],
    );
    let plugin = Plugin {
        state: root.join("state"),
        host: socket.clone(),
    };

    // Daemon: ensured by the CLI, health honest about cooperative receipt.
    // No recipe declares native-verified receipt: a harness is never
    // `supported`. An admitted one is `cooperative`, and the cooperative
    // receipt basis is then a note, never a limitation.
    let first = plugin.ensure();
    for harness in ["claude", "codex"] {
        assert_ne!(first["harness"][harness], "supported", "{first}");
    }
    let receipt_line = serde_json::json!(herdr_threads::daemon::health::COOPERATIVE_RECEIPT_LINE);
    assert!(
        !first["limitations"]
            .as_array()
            .unwrap()
            .contains(&receipt_line),
        "{first}"
    );
    if first["harness"]["claude"] == "cooperative" || first["harness"]["codex"] == "cooperative" {
        assert!(
            first["notes"].as_array().unwrap().contains(&receipt_line),
            "{first}"
        );
    }

    // Seats exist before any agent: ordinary discovery resolves empty panes.
    let a = plugin.ok(None, &["seat", "resolve", "--pane", "w1:p1"]);
    let b = plugin.ok(None, &["seat", "resolve", "--pane", "w1:p2"]);
    let (a, b) = (
        a.as_str().unwrap().to_owned(),
        b.as_str().unwrap().to_owned(),
    );
    let author = Caller {
        seat: &a,
        pane: "w1:p1",
        role: "top-level",
    };
    let top = Caller {
        seat: &b,
        pane: "w1:p2",
        role: "top-level",
    };
    let child = Caller {
        seat: &b,
        pane: "w1:p2",
        role: "subagent",
    };
    assert_eq!(
        plugin.kind(Some(author), &["check-in", "--lifecycle-event", "a-start"]),
        "checked_in"
    );

    // Prelaunch: thread, invitation and an ordinary required-ACK assignment.
    let topic = "integration sweep handoff";
    let thread = plugin.ok(Some(author), &["thread", "create", "--topic", topic]);
    let thread = thread.as_str().unwrap().to_owned();
    plugin.ok(Some(author), &["invite", &thread, "--seat", &b]);
    let handoff = plugin.ok(
        Some(author),
        &[
            "send",
            &thread,
            "--body",
            "assignment: run the sweep",
            "--require-ack",
            &b,
        ],
    );
    let handoff = handoff.as_str().unwrap().to_owned();
    let prelaunch = plugin
        .pending_for(&b, &handoff)
        .expect("prelaunch obligation");
    assert!(
        prelaunch["deadline"].is_null() && prelaunch["available_at"].is_null(),
        "no eligible occupant yet, so the receipt timer has not started: {prelaunch}"
    );

    // The handoff survives a daemon restart before the recipient launches.
    plugin.ok(None, &["daemon", "stop"]);
    let second = plugin.ensure();
    assert_ne!(second["boot_id"], first["boot_id"]);
    assert_eq!(plugin.pending_for(&b, &handoff), Some(prelaunch.clone()));
    assert_eq!(membership(&plugin, author, &thread, &b), "invited");

    // Discovery at the recipient's first top-level check-in.
    let checked_in = plugin.ok(Some(top), &["check-in", "--lifecycle-event", "b-start"]);
    let offered = checked_in["inbox"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["thread"] == thread.as_str())
        .cloned()
        .unwrap_or_else(|| panic!("check-in must offer the handoff thread: {checked_in}"));
    assert_eq!(offered["invitations"], 1, "{offered}");
    assert_eq!(offered["pending_receipts"], 1, "{offered}");
    let inbox = plugin.ok(Some(top), &["inbox"]);
    assert!(inbox.to_string().contains(&thread), "{inbox}");
    let started = plugin.pending_for(&b, &handoff).expect("still pending");
    assert!(
        started["deadline"].as_u64().is_some() && started["available_at"].as_u64().is_some(),
        "the verified check-in starts the timer and never ACKs: {started}"
    );

    // Topic/history reads confer nothing and settle nothing.
    let detail = plugin.ok(Some(top), &["thread", "show", &thread]);
    assert_eq!(detail["goal_data"], topic, "{detail}");
    assert_eq!(detail["pending_receipt_count"], 1, "{detail}");
    let history = plugin.ok(Some(top), &["read", &thread]);
    assert!(
        history["items"].as_array().unwrap().iter().any(|item| {
            item["message"] == handoff.as_str()
                && item["preview_data"] == "assignment: run the sweep"
        }),
        "{history}"
    );
    plugin.ok(Some(child), &["read", &thread]);
    assert_eq!(plugin.pending_for(&b, &handoff), Some(started.clone()));
    assert_eq!(membership(&plugin, top, &thread, &b), "invited");

    // A child context can read but cannot ACK or accept.
    for args in [
        &["ack", handoff.as_str()][..],
        &["accept", thread.as_str()][..],
    ] {
        let (code, _, stderr) = plugin.command(Some(child), args);
        assert_eq!(code, 4, "child {args:?} must be refused: {stderr}");
        assert!(stderr.contains("only top-level agents"), "{stderr}");
    }
    assert_eq!(plugin.pending_for(&b, &handoff), Some(started.clone()));
    assert_eq!(membership(&plugin, top, &thread, &b), "invited");

    // A started deadline is not reset by another restart.
    plugin.ok(None, &["daemon", "stop"]);
    let third = plugin.ensure();
    assert_ne!(third["boot_id"], second["boot_id"]);
    assert_eq!(plugin.pending_for(&b, &handoff), Some(started));

    // Explicit top-level ACK (idempotent), then separate acceptance.
    let acked = plugin.ok(Some(top), &["ack", &handoff]);
    assert_eq!(acked["acknowledged"], json!([handoff]));
    let again = plugin.ok(Some(top), &["ack", &handoff]);
    assert_eq!(again["acknowledged"], json!([]));
    assert_eq!(again["already_acknowledged"], json!([handoff]));
    assert_eq!(plugin.pending_for(&b, &handoff), None);
    assert_eq!(
        membership(&plugin, top, &thread, &b),
        "invited",
        "an ACK is not acceptance"
    );
    assert_eq!(plugin.kind(Some(top), &["accept", &thread]), "accepted");
    assert_eq!(membership(&plugin, top, &thread, &b), "joined");

    // After a daemon restart availability needs a fresh registration: the
    // agent's next hook check-in (a turn boundary) supplies it.
    assert_eq!(plugin.kind(Some(top), &["check-in"]), "checked_in");

    // Overdue warning versus quiet success.
    let quiet = plugin.ok(
        Some(author),
        &[
            "send",
            &thread,
            "--body",
            "quiet",
            "--require-ack",
            &b,
            "--deadline",
            "600",
        ],
    );
    let quiet = quiet.as_str().unwrap().to_owned();
    let late = plugin.ok(
        Some(author),
        &[
            "send",
            &thread,
            "--body",
            "late",
            "--require-ack",
            &b,
            "--deadline",
            "1",
        ],
    );
    let late = late.as_str().unwrap().to_owned();
    plugin.ok(Some(top), &["ack", &quiet]);
    let late_deadline = plugin.pending_for(&b, &late).unwrap()["deadline"]
        .as_u64()
        .expect("a joined, registered recipient starts its timer at send");
    wait_until(
        "the scheduler's overdue warning",
        Duration::from_secs(40),
        || !plugin.warnings(&b).is_empty(),
    );
    assert!(utc_ms() > late_deadline);
    let warnings = plugin.warnings(&b);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let subjects = overdue_subjects(&plugin);
    assert_eq!(
        subjects,
        vec![format!("overdue_receipt:{late}:{b}")],
        "only the missed deadline is overdue; the timely ACK stays quiet"
    );
    assert!(plugin.pending_for(&b, &late).unwrap()["overdue"] == true);
    let settled = plugin.ok(Some(top), &["ack", &late]);
    assert_eq!(
        settled["acknowledged"],
        json!([late]),
        "a late ACK stays valid"
    );
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        plugin.warnings(&b),
        warnings,
        "late settlement adds no warning"
    );

    // Leave keeps earlier obligations and stops future fanout.
    let kept = plugin.ok(
        Some(author),
        &["send", &thread, "--body", "kept", "--require-ack", &b],
    );
    let kept = kept.as_str().unwrap().to_owned();
    assert_eq!(plugin.kind(Some(top), &["leave", &thread]), "left");
    assert!(
        plugin.pending_for(&b, &kept).is_some(),
        "leave keeps the obligation"
    );
    let after = plugin.ok(Some(author), &["send", &thread, "--body", "after leave"]);
    assert!(plugin.pending_for(&b, after.as_str().unwrap()).is_none());
    let (code, _, stderr) = plugin.command(
        Some(author),
        &["send", &thread, "--body", "x", "--require-ack", &b],
    );
    assert_eq!(
        code, 2,
        "a left seat is not an explicit recipient: {stderr}"
    );
    // A left seat can be reinvited: a new membership episode, not a reset.
    plugin.ok(Some(author), &["invite", &thread, "--seat", &b]);
    let episode = |plugin: &Plugin| {
        plugin.ok(Some(top), &["thread", "show", &thread])["participants"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["seat"] == b.as_str())
            .map(|item| item["episode"].clone())
            .unwrap()
    };
    assert_eq!(membership(&plugin, top, &thread, &b), "invited");
    assert_eq!(episode(&plugin), 2);

    // Archive refuses discussion but keeps and settles obligations.
    assert_eq!(plugin.kind(Some(author), &["archive", &thread]), "archived");
    let (code, _, stderr) = plugin.command(Some(author), &["send", &thread, "--body", "x"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("archived"), "{stderr}");
    assert!(
        plugin.pending_for(&b, &kept).is_some(),
        "archive keeps the obligation"
    );
    let late_on_archive = plugin.ok(Some(top), &["ack", &kept]);
    assert_eq!(late_on_archive["acknowledged"], json!([kept]));
    // The pending reinvitation is still accepted late on the archived thread.
    assert_eq!(plugin.kind(Some(top), &["accept", &thread]), "accepted");
    assert_eq!(membership(&plugin, top, &thread, &b), "joined");
    assert_eq!(plugin.kind(Some(author), &["reopen", &thread]), "reopened");
    let reopened = plugin.ok(Some(author), &["send", &thread, "--body", "reopened"]);
    let pending: Vec<Value> = plugin.pending(&b);
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(
        pending[0]["message"], reopened,
        "the rejoined seat is in the deciding-time fanout again; nothing else is owed"
    );
    assert_eq!(
        plugin.warnings(&b).len(),
        1,
        "no warning beyond the missed deadline"
    );
}

#[test]
fn stand_in_herdr_scripts_pane_agent_present_absent_mismatched_and_error() {
    use herdr_threads::{
        host::native::NativeCli,
        ports::HostPort,
        protocol::{
            ids::HostTargetId,
            results::ErrorCode,
            time::{CallBudget, Cancellation, Clock, MonoInstant},
        },
    };
    let root = PathBuf::from(format!(
        "/private/tmp/htsweep-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let _scratch = Scratch(root.clone());
    let socket = root.join("herdr.sock");
    let mut failing = pane("w1:p4", "term-d");
    failing["agent_get_error"] = json!("permission_denied");
    let _host = FakeHost::start(
        &socket,
        vec![
            agent_pane("w1:p1", "term-a", "claude", Some("s-1")),
            pane("w1:p2", "term-b"),
            agent_pane("w1:p3", "term-c", "codex", Some("other")),
            failing,
        ],
    );
    let clock: Arc<dyn Clock> = Arc::new(herdr_threads::app::SystemClock::new());
    let cli = NativeCli::new(socket, Arc::clone(&clock));
    let observe = |pane: &str| {
        cli.observe_pane_agent(
            &HostTargetId::new(pane),
            &herdr_threads::ports::HostCallContext {
                budget: CallBudget {
                    deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
                    cancellation: Cancellation::default(),
                },
                expected_boot: None,
                expected_epoch: None,
            },
        )
    };
    let present = observe("w1:p1").unwrap().unwrap();
    assert_eq!(present.kind.as_deref(), Some("claude"));
    assert_eq!(present.agent_session.as_deref(), Some("s-1"));
    assert_eq!(observe("w1:p2").unwrap(), None);
    let mismatched = observe("w1:p3").unwrap().unwrap();
    assert_eq!(mismatched.kind.as_deref(), Some("codex"));
    assert_ne!(mismatched.agent_session.as_deref(), Some("s-1"));
    assert_eq!(mismatched.agent_session.as_deref(), Some("other"));
    assert_eq!(observe("w1:p4").unwrap_err().code, ErrorCode::Unauthorized);
}
