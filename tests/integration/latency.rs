//! ht-4is.6.7: read-only requests stay fast under a multi-agent party with a
//! slow Herdr host. Five stand-in seats check in, send required-ACK messages
//! and ACK them concurrently through the installed executable while every
//! host call (`ping`, `session.snapshot`, `pane.get`, `agent.get`,
//! `agent.prompt`) is slow, up to 1.5 s, and due wakes are prompted. History
//! and Health latency is measured end to end (including process start); a
//! follower running far longer than one request budget must keep printing
//! and never report a lost daemon. Every wait is bounded.

use super::sweep::Scratch;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::DirBuilderExt, net::UnixListener},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::channel,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
/// How long the slow host takes per method: Herdr reads under load and a
/// wake prompt that takes its time to be accepted, each still inside the
/// adapter's own per-call limit (beyond it the host lane honestly reports
/// the host unavailable and holds the mappings, which is not latency).
fn host_delay(method: &str) -> Duration {
    Duration::from_millis(match method {
        "ping" => 200,
        "session.snapshot" => 1_200,
        "pane.get" | "agent.get" => 600,
        "agent.prompt" => 1_500,
        _ => 300,
    })
}

fn agent_pane(id: &str, terminal: &str, label: &str) -> Value {
    json!({"pane_id":id,"terminal_id":terminal,"workspace_id":"w1","tab_id":format!("w1:t{id}"),
           "focused":false,"agent_status":"idle","agent":"claude","label":label,"revision":3})
}

/// A private Herdr endpoint that answers each connection on its own thread
/// after `HOST_DELAY`, like a busy real host serving concurrent clients.
struct SlowHost {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    calls: Arc<Mutex<Vec<String>>>,
    /// Set once the party starts; setup runs against a prompt host.
    slow: Arc<AtomicBool>,
}
impl SlowHost {
    fn start(socket: &Path, panes: Vec<Value>) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let slow = Arc::new(AtomicBool::new(false));
        let (stopped, seen, slowed) = (stop.clone(), calls.clone(), slow.clone());
        let panes = Arc::new(panes);
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("slow host accept: {error}"),
                };
                let (panes, seen, slowed) = (panes.clone(), seen.clone(), slowed.clone());
                std::thread::spawn(move || {
                    // A client that already hung up makes these fail (EINVAL on
                    // macOS); drop that connection (end this per-connection thread).
                    if stream.set_nonblocking(false).is_err()
                        || stream
                            .set_read_timeout(Some(Duration::from_secs(10)))
                            .is_err()
                    {
                        return;
                    }
                    let mut line = String::new();
                    if BufReader::new(&mut stream).read_line(&mut line).is_err() || line.is_empty()
                    {
                        return;
                    }
                    let Ok(request) = serde_json::from_str::<Value>(&line) else {
                        return;
                    };
                    let method = request["method"].as_str().unwrap_or_default().to_owned();
                    seen.lock().unwrap().push(method.clone());
                    if slowed.load(Ordering::SeqCst) {
                        std::thread::sleep(host_delay(&method));
                    }
                    let id = request["id"].clone();
                    let find = |key: &str| {
                        panes
                            .iter()
                            .find(|pane| pane["pane_id"] == request["params"][key])
                            .cloned()
                    };
                    let reply = match method.as_str() {
                        "ping" => {
                            json!({"id":id,"result":{"type":"pong","version":"0.9.1","protocol":22}})
                        }
                        "session.snapshot" => json!({"id":id,"result":{"type":"session_snapshot",
                            "snapshot":{"version":"0.9.1","protocol":22,"panes":*panes,
                                        "agents":[],"tabs":[],"workspaces":[],"layouts":[]}}}),
                        "pane.get" => match find("pane_id") {
                            Some(pane) => {
                                json!({"id":id,"result":{"type":"pane_info","pane":pane}})
                            }
                            None => {
                                json!({"id":id,"error":{"code":"pane_not_found","message":"pane not found"}})
                            }
                        },
                        "agent.get" => match find("target") {
                            Some(agent) => {
                                json!({"id":id,"result":{"type":"agent_info","agent":agent}})
                            }
                            None => {
                                json!({"id":id,"error":{"code":"agent_not_found","message":"no agent"}})
                            }
                        },
                        "agent.prompt" => match find("target") {
                            Some(agent) => {
                                json!({"id":id,"result":{"type":"agent_prompted","agent":agent}})
                            }
                            None => {
                                json!({"id":id,"error":{"code":"agent_not_found","message":"no agent"}})
                            }
                        },
                        _ => {
                            json!({"id":id,"error":{"code":"agent_not_found","message":"no agent in pane"}})
                        }
                    };
                    let _ = writeln!(stream, "{reply}");
                });
            }
        });
        Self {
            stop,
            worker: Some(worker),
            calls,
            slow,
        }
    }
}
impl Drop for SlowHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Plugin {
    state: PathBuf,
    host: PathBuf,
}
impl Plugin {
    fn command(&self) -> Command {
        let mut command = Command::new(BIN);
        command.envs([herdr_threads::daemon::lifecycle::test_owner_env()]);
        command
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.host)
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("HERDR_ENV")
            .env("NO_COLOR", "1");
        command
    }
    fn run(&self, agent: Option<(&str, &str)>, args: &[&str]) -> (bool, Value, String) {
        let mut command = self.command();
        command.arg("--json");
        if let Some((seat, target)) = agent {
            command.args([
                "--cooperative-seat",
                seat,
                "--cooperative-target",
                target,
                "--cooperative-harness",
                "claude",
                "--cooperative-role",
                "top-level",
            ]);
        }
        let output = command.args(args).output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let value = serde_json::from_str::<Value>(&stdout).unwrap_or(Value::Null)["result"].clone();
        (output.status.success(), value, stderr)
    }
    fn ok(&self, agent: Option<(&str, &str)>, args: &[&str]) -> Value {
        let (ok, value, stderr) = self.run(agent, args);
        assert!(ok, "herdr-threads {args:?} failed: {stderr}{value}");
        value
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.command().args(["daemon", "stop"]).output();
    }
}

fn percentile(samples: &mut [Duration], pct: usize) -> Duration {
    samples.sort();
    samples[(samples.len() * pct / 100).min(samples.len() - 1)]
}

#[test]
fn history_and_health_stay_fast_under_a_party_with_a_slow_host() {
    let root = PathBuf::from(format!(
        "/private/tmp/htlat-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let _scratch = Scratch(root.clone());
    let socket = root.join("herdr.sock");
    let names = ["alice", "hatter", "hare", "dormouse", "queen"];
    let panes: Vec<(String, Value)> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let id = format!("w1:p{}", i + 1);
            let pane = agent_pane(&id, &format!("term-{i}"), name);
            (id, pane)
        })
        .collect();
    let host = SlowHost::start(
        &socket,
        panes.iter().map(|(_, pane)| pane.clone()).collect(),
    );
    let plugin = Arc::new(Plugin {
        state: root.join("state"),
        host: socket,
    });
    plugin.ok(None, &["daemon", "ensure"]);
    let seats: Vec<(String, String)> = panes
        .iter()
        .map(|(id, _)| {
            let seat = plugin.ok(None, &["seat", "resolve", "--pane", id])["data"]
                .as_str()
                .unwrap()
                .to_owned();
            (seat, id.clone())
        })
        .collect();
    let agent = |i: usize| (seats[i].0.as_str(), seats[i].1.as_str());
    for i in 0..seats.len() {
        plugin.ok(
            Some(agent(i)),
            &["check-in", "--lifecycle-event", "agent-start"],
        );
    }
    let thread = plugin.ok(
        Some(agent(0)),
        &["thread", "create", "--topic", "tea party"],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    for (i, (seat, _)) in seats.iter().enumerate().skip(1) {
        plugin.ok(Some(agent(0)), &["invite", &thread, "--seat", seat]);
        plugin.ok(Some(agent(i)), &["accept", &thread]);
    }

    host.slow.store(true, Ordering::SeqCst);
    // A person following the thread for the whole party.
    let mut follower = plugin
        .command()
        .args(["read", &thread, "--follow", "--human", "--recent", "1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let follower_out = follower.stdout.take().unwrap();
    let (tx, follower_lines) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(follower_out).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    // The party: each seat checks in, sends a required-ACK message to the
    // next seat and ACKs whatever it owes, repeatedly.
    let party_for = Duration::from_secs(14);
    let stop = Arc::new(AtomicBool::new(false));
    let sent = Arc::new(AtomicUsize::new(0));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut party = Vec::new();
    for i in 0..seats.len() {
        let (plugin, seats, thread) = (plugin.clone(), seats.clone(), thread.clone());
        let (stop, sent, failures) = (stop.clone(), sent.clone(), failures.clone());
        party.push(std::thread::spawn(move || {
            let me = (seats[i].0.as_str(), seats[i].1.as_str());
            let next = &seats[(i + 1) % seats.len()].0;
            let mut round = 0;
            while !stop.load(Ordering::SeqCst) {
                round += 1;
                let body = format!("round {round} from {i}");
                let steps: [&[&str]; 2] = [
                    &["check-in", "--lifecycle-event", "turn-end"],
                    &["send", &thread, "--body", &body, "--require-ack", next],
                ];
                for args in steps {
                    let (ok, value, stderr) = plugin.run(Some(me), args);
                    if !ok {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("{args:?}: {stderr}{value}"));
                    } else if args[0] == "send" {
                        sent.fetch_add(1, Ordering::SeqCst);
                    }
                }
                let pending =
                    plugin.run(None, &["pending-receipts", "--seat", me.0, "--limit", "20"]);
                if let Some(items) = pending.1["data"]["items"].as_array() {
                    for item in items.iter().take(3) {
                        if let Some(message) = item["message"].as_str() {
                            let _ = plugin.run(Some(me), &["ack", message]);
                        }
                    }
                }
            }
        }));
    }

    // Probe read-only latency while the party runs.
    let started = Instant::now();
    let mut history = Vec::new();
    let mut health = Vec::new();
    while started.elapsed() < party_for {
        let begin = Instant::now();
        let (ok, value, stderr) =
            plugin.run(None, &["read", &thread, "--after", "0", "--limit", "50"]);
        assert!(ok, "history failed under load: {stderr}{value}");
        history.push(begin.elapsed());
        let begin = Instant::now();
        let (ok, value, stderr) = plugin.run(None, &["daemon", "health"]);
        assert!(ok, "health failed under load: {stderr}{value}");
        health.push(begin.elapsed());
        std::thread::sleep(Duration::from_millis(100));
    }
    stop.store(true, Ordering::SeqCst);
    for worker in party {
        worker.join().unwrap();
    }
    for failure in failures.lock().unwrap().iter().take(8) {
        eprintln!("party failure: {failure}");
    }
    // A last append after the party: the follower, now running far longer
    // than one request budget, still prints it.
    host.slow.store(false, Ordering::SeqCst);
    eprintln!(
        "health after the party: {}",
        plugin.run(None, &["daemon", "health"]).1
    );
    // A slow host can leave seats unresolved until a prompt snapshot
    // reconfirms them; that is the host lane's honest state, not latency.
    let resend = Instant::now() + Duration::from_secs(30);
    loop {
        let (ok, value, stderr) = plugin.run(
            Some(agent(0)),
            &["send", &thread, "--body", "Off with their heads!"],
        );
        if ok {
            break;
        }
        assert!(Instant::now() < resend, "final send: {stderr}{value}");
        let _ = plugin.run(
            Some(agent(0)),
            &["check-in", "--lifecycle-event", "turn-end"],
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Ok(line) = follower_lines.recv_timeout(Duration::from_millis(200)) {
            let done = line.contains("Off with their heads!");
            seen.push(line);
            if done {
                break;
            }
        }
    }
    let _ = follower.kill();
    let _ = follower.wait();

    let (h50, h95, hmax) = (
        percentile(&mut history, 50),
        percentile(&mut history, 95),
        *history.iter().max().unwrap(),
    );
    let (k50, k95) = (percentile(&mut health, 50), percentile(&mut health, 95));
    let host_calls = host.calls.lock().unwrap().clone();
    let prompts = host_calls.iter().filter(|m| *m == "agent.prompt").count();
    eprintln!(
        "latency: history n={} p50={h50:?} p95={h95:?} max={hmax:?}; health n={} p50={k50:?} p95={k95:?}; \
         sends={} host calls={} (agent.prompt {prompts}); party failures={}",
        history.len(),
        health.len(),
        sent.load(Ordering::SeqCst),
        host_calls.len(),
        failures.lock().unwrap().len(),
    );
    assert!(sent.load(Ordering::SeqCst) >= 5, "the party made progress");
    assert!(
        h95 < Duration::from_millis(1_000),
        "history p95 {h95:?} (p50 {h50:?}, max {hmax:?})"
    );
    assert!(k95 < Duration::from_millis(1_000), "health p95 {k95:?}");
    assert!(
        seen.iter()
            .any(|line| line.contains("Off with their heads!")),
        "the long-running follower kept printing: {seen:?}"
    );
    assert!(
        !seen.iter().any(|line| line.contains("lost the daemon")),
        "no reconnect notice while the daemon stayed up: {seen:?}"
    );
}
