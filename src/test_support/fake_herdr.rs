//! A fake Herdr server for cluster tests (ht-5fr.11): a Unix endpoint served by
//! a thread of the test process, answering the protocol subset a detached
//! daemon uses to observe stand-in Claude panes and to wake them.
//!
//! Responsibility
//! - Owns: [`FakeHerdr`], which listens on a socket path, serves `ping`,
//!   `session.snapshot`, `pane.get`, `pane.process_info`, `agent.get`,
//!   `agent.read` and `agent.prompt` for a set of idle Claude panes, and
//!   records every accepted `agent.prompt` as a wake marker.
//! - Hides: the wire shapes, the per-pane stand-in session naming and the
//!   foreground-process fingerprint the composer wake matches.
//! - Must not: know a fact kind or a delivery rule; it only answers the host
//!   protocol and records what was typed into which pane.
//! - Role: test fixture for `multinode::Cluster` (a node's Herdr host). A
//!   daemon attaches it lazily when a CLI call names its socket as
//!   `--host-endpoint`, exactly as it attaches a real server.

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

/// The stand-in harness session of the agent in `pane` on the fake server
/// `label`: each pane's agent is a process of its own.
pub fn session_of(label: &str, pane: &str) -> String {
    format!("{label}-{pane}")
}

struct World {
    label: String,
    panes: Mutex<Vec<Value>>,
    prompts: Mutex<Vec<(String, String)>>,
}

impl World {
    fn pane(&self, id: &Value) -> Option<Value> {
        self.panes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|pane| &pane["pane_id"] == id)
            .cloned()
    }

    fn reply(&self, request: &Value) -> Value {
        let id = request["id"].clone();
        let params = &request["params"];
        let missing = |code: &str| json!({"id":id,"error":{"code":code,"message":code}});
        match request["method"].as_str().unwrap_or_default() {
            "ping" => json!({"id":id,"result":{"type":"pong","version":"0.9.1","protocol":22}}),
            "session.snapshot" => {
                let panes = self.panes.lock().unwrap_or_else(|e| e.into_inner()).clone();
                json!({"id":id,"result":{"type":"session_snapshot",
                    "snapshot":{"version":"0.9.1","protocol":22,"panes":panes,
                                "agents":[],"tabs":[{"tab_id":"w1:t1","workspace_id":"w1"}],
                                "workspaces":[{"workspace_id":"w1"}],"layouts":[]}}})
            }
            "pane.get" => match self.pane(&params["pane_id"]) {
                Some(pane) => json!({"id":id,"result":{"type":"pane_info","pane":pane}}),
                None => missing("pane_not_found"),
            },
            "pane.current" => match self.pane(&params["caller_pane_id"]) {
                Some(pane) => json!({"id":id,"result":{"type":"pane_current","pane":pane}}),
                None => missing("pane_not_found"),
            },
            // The pane's foreground process is its agent's stand-in process: the
            // composer wake types only to the binding with that fingerprint pid.
            "pane.process_info" => match self.pane(&params["pane_id"]) {
                Some(pane) => {
                    let session = session_of(&self.label, pane["pane_id"].as_str().unwrap_or(""));
                    let pid =
                        crate::test_support::isolation::stand_in_fingerprint("claude", &session)
                            .pid;
                    json!({"id":id,"result":{"type":"pane_process_info","process_info":{
                        "pane_id":pane["pane_id"],"shell_pid":1,
                        "foreground_processes":[{"pid":pid,"name":"claude","argv0":"claude"}]}}})
                }
                None => missing("pane_not_found"),
            },
            "agent.get" => match self.pane(&params["target"]) {
                Some(agent) => json!({"id":id,"result":{"type":"agent_info","agent":agent}}),
                None => missing("agent_not_found"),
            },
            "agent.prompt" => match self.pane(&params["target"]) {
                Some(agent) => {
                    let text = params["text"]
                        .as_str()
                        .or_else(|| params["prompt"].as_str())
                        .unwrap_or_default()
                        .to_owned();
                    self.prompts
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push((agent["pane_id"].as_str().unwrap_or("").to_owned(), text));
                    json!({"id":id,"result":{"type":"agent_prompted","agent":agent}})
                }
                None => missing("agent_not_found"),
            },
            "agent.read" => match self.pane(&params["target"]) {
                Some(pane) => {
                    let text = include_str!(
                        "../../docs/evidence/poke-spike/captures/claude-q1-empty.read-detection.txt"
                    );
                    json!({"id":id,"result":{"type":"pane_read","read":{
                        "pane_id":pane["pane_id"],"source":"detection","text":text}}})
                }
                None => missing("agent_not_found"),
            },
            _ => missing("agent_not_found"),
        }
    }
}

/// One fake Herdr server. Stopped (and its socket withdrawn) on drop.
pub struct FakeHerdr {
    socket: PathBuf,
    world: Arc<World>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl FakeHerdr {
    /// Serves `panes` (idle Claude agents) at `socket`. `label` names the
    /// server in the stand-in session of each pane.
    pub fn start(label: &str, socket: &Path, panes: &[&str]) -> Self {
        let listener = UnixListener::bind(socket)
            .unwrap_or_else(|e| panic!("bind the fake Herdr at {}: {e}", socket.display()));
        listener.set_nonblocking(true).unwrap();
        let panes = panes
            .iter()
            .enumerate()
            .map(|(i, id)| {
                json!({"pane_id":id,"terminal_id":format!("term-{label}-{i}"),
                       "workspace_id":"w1","tab_id":"w1:t1",
                       "focused":false,"agent_status":"idle","agent":"claude",
                       "label":id,"revision":3})
            })
            .collect();
        let world = Arc::new(World {
            label: label.to_owned(),
            panes: Mutex::new(panes),
            prompts: Mutex::new(Vec::new()),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (serving, stopped) = (Arc::clone(&world), Arc::clone(&stop));
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => continue,
                };
                let world = Arc::clone(&serving);
                std::thread::spawn(move || {
                    // macOS accept(2) inherits O_NONBLOCK; a client that already
                    // hung up makes these fail: drop that connection.
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
                    if let Ok(request) = serde_json::from_str::<Value>(&line) {
                        let _ = writeln!(stream, "{}", world.reply(&request));
                    }
                });
            }
        });
        Self {
            socket: socket.to_path_buf(),
            world,
            stop,
            worker: Some(worker),
        }
    }

    /// The endpoint a CLI call names with `--host-endpoint`.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The server's label (see [`session_of`]).
    pub fn label(&self) -> &str {
        &self.world.label
    }

    /// Every `(pane, text)` an `agent.prompt` was accepted for, in order: the
    /// wake markers the daemon typed.
    pub fn prompts(&self) -> Vec<(String, String)> {
        self.world
            .prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The wake markers typed into `pane`.
    pub fn prompts_for(&self, pane: &str) -> usize {
        self.prompts().iter().filter(|(p, _)| p == pane).count()
    }

    /// Sets the Herdr agent status (`idle`, `working`, ...) of `pane`.
    pub fn set_status(&self, pane: &str, status: &str) {
        for entry in self
            .world
            .panes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter_mut()
        {
            if entry["pane_id"] == pane {
                entry["agent_status"] = json!(status);
            }
        }
    }
}

impl Drop for FakeHerdr {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}
