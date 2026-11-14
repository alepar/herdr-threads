//! ht-vxdp.6: the opencode native gate (opencode design D12 tier 2), opt-in and model-free. It
//! runs only when `HT_OPENCODE_BIN` names an opencode executable and skips otherwise (CI has no
//! opencode).
//!
//! The real binary, the plugin installed by the public `herdr-threads setup opencode` and one
//! isolated daemon (the named-seats rig) work together:
//! - drive mode `opencode serve` on loopback, prompted over its HTTP API: plugins load in the
//!   server process, and the process stays up and idle between turns, so the plugin's channel
//!   wake can be observed (`opencode run` exits after one turn; the TUI needs a terminal and is
//!   reached through the same HTTP API with `--port`);
//! - the model is an in-process scripted OpenAI-compatible server on an ephemeral loopback port;
//!   `RUNBASH: <cmd>` in the last user text answers with a `bash` tool call, `RUNTASK: <cmd>` with
//!   a `task` tool call whose prompt is `RUNBASH: <cmd>`; every request body is kept, so what
//!   reached the model's context is asserted exactly;
//! - the seat is a named seat (`HERDR_THREADS_SEAT_NAME`), so no Herdr pane or server is needed;
//!   the sender is a second named seat (person seats need a Herdr pane);
//! - HOME, the four `XDG_*_HOME`, `TMPDIR` and `OPENCODE_CONFIG_DIR` are scratch; no credential
//!   exists. opencode's npm registry is the mock (`NPM_CONFIG_REGISTRY`), which answers 404, so
//!   its background `@opencode-ai/plugin` install fails at once. When `unshare -rn` works the
//!   test re-executes itself in a private network namespace (loopback only; set
//!   `HT_OPENCODE_GATE_NETNS=0` to opt out), so nothing else is reachable either; without it
//!   every proxy variable points at a closed loopback port.
//!
//! The opencode version is only printed as evidence, never compared.

#[allow(dead_code)]
#[path = "named_seats/rig.rs"]
mod rig;

use herdr_threads::test_support::spawn::{self, OwnedChild, SpawnOwned};
use rig::{BIN, Rig, run, sent_message, wait_for};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

const BIN_ENV: &str = "HT_OPENCODE_BIN";
const NETNS_ENV: &str = "HT_OPENCODE_GATE_NETNS";
const WAKE_UP: &str = "herdr-threads: wake up";
const SEAT_NAME: &str = "oc-gate";
/// A non-default primary agent; its prompt marks every model call it runs.
const AGENT: &str = "gatekeeper";
const AGENT_PROMPT: &str = "GATE-AGENT-PROMPT: you are the gatekeeper agent.";

// ------------------------------------------------------------------ the scripted model

/// One scripted OpenAI-compatible endpoint; `requests` holds every chat-completions body.
struct MockModel {
    port: u16,
    requests: Arc<Mutex<Vec<Value>>>,
}

fn text_of(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The scripted answer to one request body: a tool call `(name, arguments)` or a text.
fn script(body: &Value) -> Result<(String, Value), String> {
    let tools: Vec<&str> = body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["function"]["name"].as_str())
                .collect()
        })
        .unwrap_or_default();
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let Some(last) = messages.last() else {
        return Err("MOCK-EMPTY".into());
    };
    if tools.is_empty() {
        return Err("Gate title".into());
    }
    if last["role"] == "tool" {
        return Err("MOCK-TOOL-RESULT-SEEN".into());
    }
    let text = text_of(last);
    let line_after = |marker: &str| {
        text.lines().find_map(|line| {
            line.split_once(marker)
                .map(|(_, rest)| rest.trim().to_owned())
        })
    };
    if let Some(command) = line_after("RUNTASK:").filter(|_| tools.contains(&"task")) {
        return Ok((
            "task".into(),
            json!({"description": "gate child", "prompt": format!("RUNBASH: {command}"), "subagent_type": "general"}),
        ));
    }
    if let Some(command) = line_after("RUNBASH:").filter(|_| tools.contains(&"bash")) {
        return Ok((
            "bash".into(),
            json!({"command": command, "description": "gate command"}),
        ));
    }
    Err("MOCK-REPLY".into())
}

fn serve_one(mut stream: TcpStream, requests: &Mutex<Vec<Value>>, seq: &Mutex<u64>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut raw = vec![0; length];
    if reader.read_exact(&mut raw).is_err() {
        return;
    }
    if request_line.starts_with("GET") && !request_line.contains("/v1/models") {
        // opencode's npm registry (`NPM_CONFIG_REGISTRY` points here): every package is absent,
        // so its background `@opencode-ai/plugin` install fails at once, with no network.
        let _ = write!(
            stream,
            "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}"
        );
        return;
    }
    if request_line.starts_with("GET") {
        let body = r#"{"object":"list","data":[{"id":"mock-1","object":"model"}]}"#;
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        return;
    }
    let body: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let answer = script(&body);
    requests.lock().unwrap().push(body);
    let id = {
        let mut seq = seq.lock().unwrap();
        *seq += 1;
        *seq
    };
    let base = json!({"id": format!("chatcmpl-{id}"), "object": "chat.completion.chunk", "created": 0, "model": "mock-1"});
    let chunk = |choices: Value| {
        let mut event = base.clone();
        event["choices"] = choices;
        format!("data: {event}\n\n")
    };
    let mut out = String::from(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n",
    );
    match answer {
        Ok((name, arguments)) => {
            out += &chunk(
                json!([{"index": 0, "delta": {"role": "assistant", "tool_calls": [{"index": 0, "id": format!("call_{id}"), "type": "function", "function": {"name": name, "arguments": arguments.to_string()}}]}, "finish_reason": null}]),
            );
            out += &chunk(json!([{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]));
        }
        Err(text) => {
            out += &chunk(
                json!([{"index": 0, "delta": {"role": "assistant", "content": text}, "finish_reason": null}]),
            );
            out += &chunk(json!([{"index": 0, "delta": {}, "finish_reason": "stop"}]));
        }
    }
    let mut usage = base.clone();
    usage["choices"] = json!([]);
    usage["usage"] = json!({"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15});
    out += &format!("data: {usage}\n\ndata: [DONE]\n\n");
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.flush();
}

impl MockModel {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the mock model");
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seq = Arc::new(Mutex::new(0u64));
        let shared = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let requests = Arc::clone(&shared);
                let seq = Arc::clone(&seq);
                std::thread::spawn(move || serve_one(stream, &requests, &seq));
            }
        });
        Self { port, requests }
    }

    /// The bodies of the agent's model calls (with tools), not the title calls.
    fn agent_requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|body| body["tools"].as_array().is_some_and(|t| !t.is_empty()))
            .cloned()
            .collect()
    }
}

/// Every message text of `body`, joined.
fn context_of(body: &Value) -> String {
    body["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .map(text_of)
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default()
}

fn last_text(body: &Value) -> String {
    body["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .map(text_of)
        .unwrap_or_default()
}

// ------------------------------------------------------------------ opencode's HTTP API

/// One HTTP/1.1 exchange with `connection: close`; the status and the (de-chunked) body.
fn http(port: u16, method: &str, path: &str, body: Option<&Value>) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to opencode");
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .unwrap();
    let payload = body.map(Value::to_string).unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\ncontent-type: application/json\r\naccept: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
        payload.len()
    )
    .unwrap();
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .expect("read the opencode status line");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let (mut length, mut chunked) = (None, false);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).expect("read a header") == 0 || line == "\r\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse::<usize>().ok();
        }
        if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            chunked = true;
        }
    }
    // opencode keeps the connection open: read exactly the body, never to EOF.
    let mut out = Vec::new();
    if chunked {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size).expect("read a chunk size");
            let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            let mut chunk = vec![0; size + 2];
            reader.read_exact(&mut chunk).expect("read a chunk");
            out.extend_from_slice(&chunk[..size]);
        }
    } else if let Some(length) = length {
        out.resize(length, 0);
        reader.read_exact(&mut out).expect("read the body");
    }
    (status, String::from_utf8_lossy(&out).into_owned())
}

struct Opencode {
    child: Option<OwnedChild>,
    port: u16,
    directory: PathBuf,
}

impl Opencode {
    fn query(&self) -> String {
        format!("directory={}", self.directory.display())
    }

    fn get(&self, path: &str) -> Value {
        let sep = if path.contains('?') { '&' } else { '?' };
        let (status, body) = http(
            self.port,
            "GET",
            &format!("{path}{sep}{}", self.query()),
            None,
        );
        assert_eq!(status, 200, "GET {path}: {body}");
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("GET {path}: {e}: {body}"))
    }

    fn post(&self, path: &str, body: &Value) -> Value {
        let (status, text) = http(
            self.port,
            "POST",
            &format!("{path}?{}", self.query()),
            Some(body),
        );
        assert!(
            status == 200 || status == 204,
            "POST {path}: {status} {text}"
        );
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    /// One user turn in `session`, returned when the turn has finished.
    fn prompt(&self, session: &str, text: &str) -> Value {
        self.post(
            &format!("/session/{session}/message"),
            &json!({"parts": [{"type": "text", "text": text}]}),
        )
    }

    /// The stored messages of `session` (`{info, parts}` each), oldest first.
    fn messages(&self, session: &str) -> Vec<Value> {
        self.get(&format!("/session/{session}/message"))
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// Whether no session of this process is busy (`/session/status` lists only busy ones).
    fn all_idle(&self) -> bool {
        self.get("/session/status")
            .as_object()
            .is_some_and(|map| map.is_empty())
    }
}

impl Drop for Opencode {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.stop();
        }
    }
}

// ------------------------------------------------------------------ the gate

/// Re-executes this test in a private network namespace with loopback only; `false` when
/// `unshare -rn` is unavailable (the test then runs in place).
fn reexec_without_network(test: &str) -> bool {
    let probe = spawn::command("unshare")
        .args(["-rn", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !probe.is_ok_and(|status| status.success()) {
        println!("note: unshare -rn unavailable; running with dead-proxy variables instead");
        return false;
    }
    let exe = std::env::current_exe().expect("the test executable");
    let mut command = spawn::command("unshare");
    command
        .args(["-rn", "sh", "-c", r#"ip link set lo up && exec "$@""#, "sh"])
        .arg(&exe)
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env(NETNS_ENV, "1")
        .env(BIN_ENV, std::env::var_os(BIN_ENV).unwrap());
    let mut child = command.spawn_owned().expect("spawn the namespaced test");
    let status = child.wait().expect("wait for the namespaced test");
    assert!(
        status.success(),
        "the gate failed inside its network namespace: {status}"
    );
    true
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn heartbeat(state: &Path) -> Option<Value> {
    serde_json::from_str(&read(&state.join("opencode/plugin-heartbeat.json"))).ok()
}

/// The ledger lines the plugin wrote (`HERDR_THREADS_MOD_LEDGER`).
fn ledger(path: &Path) -> Vec<Value> {
    read(path)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn delivered(path: &Path, id: &str) -> bool {
    ledger(path).iter().any(|entry| {
        entry["kind"] == "delivered"
            && entry["via"] == "append"
            && entry["ids"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|i| i == id))
    })
}

/// On a failing run, prints the files that explain it before the scratch root is removed.
struct DumpOnPanic(Vec<PathBuf>);

impl Drop for DumpOnPanic {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        for path in &self.0 {
            let mut files = vec![path.clone()];
            if path.is_dir() {
                files = std::fs::read_dir(path)
                    .map(|dir| dir.flatten().map(|e| e.path()).collect())
                    .unwrap_or_default();
            }
            for file in files {
                let text = read(&file);
                let tail: String = text
                    .chars()
                    .rev()
                    .take(6000)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                eprintln!("==== {} ====\n{tail}", file.display());
            }
        }
    }
}

fn step(what: &str) {
    eprintln!("gate step: {what}");
}

#[test]
fn opencode_native_gate() {
    let Some(opencode_bin) = std::env::var_os(BIN_ENV).filter(|v| !v.is_empty()) else {
        println!("skipped: {BIN_ENV} not set (opt-in opencode native gate)");
        return;
    };
    let opencode_bin = PathBuf::from(opencode_bin);
    assert!(
        opencode_bin.is_file(),
        "{BIN_ENV} names no file: {}",
        opencode_bin.display()
    );
    let in_netns = std::env::var_os(NETNS_ENV).is_some_and(|v| v == "1");
    if !in_netns
        && std::env::var_os(NETNS_ENV).is_none_or(|v| v != "0")
        && reexec_without_network("opencode_native_gate")
    {
        return;
    }

    let rig = Rig::standalone("oc-native");
    rig.enroll("alice", "s-alice");
    let alice = rig.seat_of("alice");
    let model = MockModel::start();

    let dir = |name: &str| {
        let path = rig.isolation.path(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let data = dir("xdg/data");
    let cache = dir("xdg/cache");
    let tmp = dir("tmp");
    let config_dir = dir("opencode-config");
    let project = dir("project");
    let ledger_file = rig.isolation.path("plugin-ledger.jsonl");
    let _dump = DumpOnPanic(vec![
        ledger_file.clone(),
        rig.isolation.path("opencode-serve.log"),
        data.join("opencode/log"),
        rig.isolation.path("daemon.stderr"),
    ]);
    let path_dirs = format!("{}:/usr/bin:/bin", rig.bin.display());
    std::os::unix::fs::symlink(BIN, rig.bin.join("herdr-threads")).unwrap();
    let opencode_env = |command: &mut std::process::Command| {
        command
            .env("PATH", &path_dirs)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CACHE_HOME", &cache)
            .env("TMPDIR", &tmp)
            .env("OPENCODE_CONFIG_DIR", &config_dir)
            .env("OPENCODE_DISABLE_AUTOUPDATE", "1")
            .env("OPENCODE_DISABLE_MODELS_FETCH", "1")
            .env("OPENCODE_DISABLE_LSP_DOWNLOAD", "1")
            .env("OPENCODE_DISABLE_SHARE", "1")
            .env("HERDR_THREADS_SEAT_NAME", SEAT_NAME)
            // The model's own `herdr-threads` commands carry no `--state-dir`: this is the
            // variable a Herdr pane gives them.
            .env("HERDR_PLUGIN_STATE_DIR", &rig.state)
            .env("HERDR_THREADS_MOD_LEDGER", &ledger_file)
            .env(
                "NPM_CONFIG_REGISTRY",
                format!("http://127.0.0.1:{}/npm/", model.port),
            );
        if !in_netns {
            for proxy in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
                command.env(proxy, "http://127.0.0.1:9");
            }
            command
                .env("NO_PROXY", "127.0.0.1,localhost")
                .env("no_proxy", "127.0.0.1,localhost");
        }
    };

    let version = {
        let mut command = rig.base(&opencode_bin);
        opencode_env(&mut command);
        command.arg("--version").current_dir(&project);
        run(command, None).stdout.trim().to_owned()
    };
    println!("evidence: opencode --version: {version}; network namespace: {in_netns}");

    step("setup");
    // 1. Setup through the public CLI into the scratch config root.
    let setup = {
        let mut command = rig.base(BIN);
        opencode_env(&mut command);
        command
            .args(["setup", "opencode", "--state-dir"])
            .arg(&rig.state)
            .arg("--host-endpoint")
            .arg(rig.isolation.path("no-herdr.sock"))
            .arg("--harness-binary")
            .arg(&opencode_bin)
            .args(["--with-permissions", "--json"]);
        run(command, None)
    };
    assert_eq!(setup.code, Some(0), "setup opencode: {setup:?}");
    let report: Value = serde_json::from_str(&setup.stdout).expect("setup JSON");
    assert_eq!(report["setup"]["installed"], true, "{report}");
    assert!(config_dir.join("plugins/herdr-threads.js").is_file());
    assert!(config_dir.join("skills/herdr-threads/SKILL.md").is_file());
    std::fs::write(
        config_dir.join("opencode.json"),
        json!({
            "$schema": "https://opencode.ai/config.json",
            "autoupdate": false,
            "share": "disabled",
            "model": "mock/mock-1",
            "small_model": "mock/mock-1",
            "provider": {"mock": {
                "npm": "@ai-sdk/openai-compatible",
                "name": "Mock",
                "options": {"baseURL": format!("http://127.0.0.1:{}/v1", model.port), "apiKey": "none"},
                "models": {"mock-1": {"name": "Mock 1", "tool_call": true}},
            }},
            "agent": {AGENT: {"mode": "primary", "prompt": AGENT_PROMPT}},
        })
        .to_string(),
    )
    .unwrap();

    // opencode serve on a free loopback port.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let server_log = rig.isolation.path("opencode-serve.log");
    let mut command = rig.base(&opencode_bin);
    opencode_env(&mut command);
    command
        .args([
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .current_dir(&project)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&server_log).unwrap())
        .stderr(Stdio::null());
    let opencode = Opencode {
        child: Some(command.spawn_owned().expect("spawn opencode serve")),
        port,
        directory: project.canonicalize().unwrap(),
    };
    let server_pid = opencode.child.as_ref().unwrap().id();
    wait_for("opencode serve listening", Duration::from_secs(60), || {
        if read(&server_log).contains("listening") {
            Ok(())
        } else {
            Err(read(&server_log))
        }
    });
    step("serve listening; bootstrap");
    // The first request for the directory bootstraps its instance, which loads the plugins.
    let _ = opencode.get("/session");
    let beat = wait_for("the plugin heartbeat", Duration::from_secs(150), || {
        heartbeat(&rig.state).ok_or_else(|| "no heartbeat".to_owned())
    });
    println!("evidence: heartbeat {beat}");
    assert_eq!(beat["role"], "active", "{beat}");
    assert_eq!(beat["launch"], true, "{beat}");
    assert_eq!(beat["pid"], json!(server_pid), "{beat}");
    for capability in ["append", "wake", "status", "session_get"] {
        assert_eq!(beat["capabilities"][capability], true, "{beat}");
    }
    let status = {
        let mut command = rig.base(BIN);
        opencode_env(&mut command);
        command
            .args(["setup-status", "opencode", "--state-dir"])
            .arg(&rig.state)
            .arg("--json");
        run(command, None)
    };
    println!("evidence: setup-status {}", status.stdout.trim());
    let doctor = {
        let mut command = rig.base(BIN);
        opencode_env(&mut command);
        command
            .arg("--state-dir")
            .arg(&rig.state)
            .args(["doctor", "--json"]);
        run(command, None)
    };
    // doctor reads the heartbeat (`heartbeat.state`, design D6).
    fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
        match value {
            Value::Object(map) => map
                .get(key)
                .or_else(|| map.values().find_map(|v| find_key(v, key))),
            Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
            _ => None,
        }
    }
    let report: Value = serde_json::from_str(&doctor.stdout)
        .unwrap_or_else(|e| panic!("doctor JSON: {e}: {doctor:?}"));
    let reported = find_key(&report, "heartbeat")
        .unwrap_or_else(|| panic!("doctor reports the plugin heartbeat: {doctor:?}"));
    println!("evidence: doctor heartbeat {reported}");
    assert_eq!(reported["state"], "loaded", "{reported}");

    step("heartbeat seen");
    // 2. The first prompts: a SessionStart check-in enrolls the session as a top-level binding.
    let session = opencode.post("/session", &json!({}))["id"]
        .as_str()
        .expect("a session id")
        .to_owned();
    assert!(session.starts_with("ses_"), "{session}");
    let mut turns = 0;
    let seat = wait_for(
        "the opencode session enrolled",
        Duration::from_secs(90),
        || {
            turns += 1;
            step(&format!("turn {turns}"));
            opencode.prompt(&session, &format!("hello {turns}"));
            let row: Option<(String, String)> = rig
                .db()
                .query_row(
                    "SELECT seat_id, observation_provenance FROM occupant_bindings \
                 WHERE harness='opencode' AND native_session=?1 AND ended_at IS NULL",
                    [&session],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            row.ok_or_else(|| format!("no binding after {turns} turns"))
        },
    );
    println!("evidence: enrolled after {turns} turn(s): {seat:?}");
    let (seat, provenance) = seat;
    assert_eq!(provenance, "cooperative_top_level");
    assert_eq!(rig.seat_of(SEAT_NAME), seat);
    let checked_in = model
        .agent_requests()
        .iter()
        .any(|body| last_text(body).contains("Your seat"));
    assert!(checked_in, "the check-in context reached the model");

    step("enrolled");
    // 3. A scripted bash call: the session variables are in its environment and its
    //    herdr-threads commands are attributed to the session.
    let marker = "OC-SENT-1";
    opencode.prompt(
        &session,
        &format!(
            "RUNBASH: env | grep '^HERDR_THREADS_OPENCODE_' | sort; herdr-threads inbox --machine; herdr-threads send --seat {alice} --body {marker}"
        ),
    );
    let tool_output = model
        .agent_requests()
        .iter()
        .rev()
        .find_map(|body| {
            let last = body["messages"].as_array()?.last()?;
            (last["role"] == "tool").then(|| text_of(last))
        })
        .expect("a bash tool result");
    println!("evidence: bash tool result:\n{tool_output}");
    assert!(tool_output.contains(&format!("HERDR_THREADS_OPENCODE_SESSION={session}")));
    assert!(tool_output.contains("HERDR_THREADS_OPENCODE_PARENT=\n"));
    assert!(tool_output.contains(&format!("HERDR_THREADS_OPENCODE_PID={server_pid}")));
    assert!(
        !tool_output.contains("session_claim_required"),
        "{tool_output}"
    );
    let alice_inbox = rig.agent("alice", "s-alice", &["inbox", "--machine"]);
    assert!(
        alice_inbox.stdout.contains(marker) && alice_inbox.stdout.contains(&seat),
        "alice got the opencode session's send from its seat: {alice_inbox:?}"
    );

    step("bash checked");
    // 4./5. A p1 message at threshold 0: appended into the idle session, no wake.
    let calls = model.agent_requests().len();
    let sent = rig.agent(
        "alice",
        "s-alice",
        &["send", "--seat", &seat, "--body", "PASSIVE-KIWI"],
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    let passive = sent_message(&sent);
    wait_for("the p1 append", Duration::from_secs(30), || {
        if delivered(&ledger_file, &passive) {
            Ok(())
        } else {
            Err(read(&ledger_file))
        }
    });
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(
        model.agent_requests().len(),
        calls,
        "a p1 append at T=0 starts no turn"
    );
    assert!(opencode.all_idle());
    opencode.prompt(&session, "next turn");
    let next = model.agent_requests();
    assert!(
        context_of(&next[calls]).contains("PASSIVE-KIWI"),
        "the appended p1 item is in the next turn's context"
    );

    step("p1 checked");
    // 4./5. An ACK request (p0): appended, the idle session woken by the plugin, the receipt
    //    settled with the push-channel provenance.
    let calls = model.agent_requests().len();
    let sent = rig.agent(
        "alice",
        "s-alice",
        &[
            "send",
            "--seat",
            &seat,
            "--require-ack",
            &seat,
            "--body",
            "ACK-MANGO",
        ],
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    let acked = sent_message(&sent);
    let woken = wait_for("the plugin's wake turn", Duration::from_secs(60), || {
        model
            .agent_requests()
            .iter()
            .skip(calls)
            .find(|body| last_text(body).starts_with(WAKE_UP))
            .cloned()
            .ok_or_else(|| read(&ledger_file))
    });
    assert!(
        context_of(&woken).contains("ACK-MANGO"),
        "the woken turn sees the appended item"
    );
    let (state, observation) = wait_for("the receipt settled", Duration::from_secs(30), || {
        rig.db()
            .query_row(
                "SELECT state, COALESCE(ack_observation,'') FROM receipt_state \
                 WHERE message_id=?1 AND seat_id=?2",
                [&acked, &seat],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(|e| e.to_string())
            .and_then(|row| {
                if row.0 == "acked" {
                    Ok(row)
                } else {
                    Err(format!("{row:?}"))
                }
            })
    });
    println!("evidence: receipt {state} {observation}");
    let observation: Value = serde_json::from_str(&observation).unwrap();
    assert_eq!(observation["action_provenance"], "cooperative_mod_delivery");
    assert_eq!(observation["via"], "append");
    wait_for("the woken turn finished", Duration::from_secs(60), || {
        if opencode.all_idle() {
            Ok(())
        } else {
            Err("busy".to_owned())
        }
    });

    step("wake checked");
    // 5b. A non-default agent (a `--agent` launch, or Tab in the TUI) is kept by the plugin's own
    //    messages: opencode runs each step with the newest user message's agent, and a message
    //    without one gets the default agent. A turn with the agent, a p1 appended mid-turn (the
    //    step after it reads the append), then a p0 woken on idle.
    let plain = |text: &str| json!({"agent": AGENT, "parts": [{"type": "text", "text": text}]});
    let calls = model.agent_requests().len();
    let (port, query) = (opencode.port, opencode.query());
    let busy_turn = {
        let body = plain("RUNBASH: sleep 6");
        let session = session.clone();
        std::thread::spawn(move || {
            http(
                port,
                "POST",
                &format!("/session/{session}/message?{query}"),
                Some(&body),
            )
        })
    };
    wait_for("the agent's tool call", Duration::from_secs(30), || {
        if model.agent_requests().len() > calls {
            Ok(())
        } else {
            Err("no model call yet".to_owned())
        }
    });
    let sent = rig.agent(
        "alice",
        "s-alice",
        &["send", "--seat", &seat, "--body", "MIDTURN-FIG"],
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    let midturn = sent_message(&sent);
    wait_for("the mid-turn append", Duration::from_secs(5), || {
        if delivered(&ledger_file, &midturn) {
            Ok(())
        } else {
            Err(read(&ledger_file))
        }
    });
    let (status, text) = busy_turn.join().expect("the busy turn");
    assert_eq!(status, 200, "{text}");
    let after_append = model
        .agent_requests()
        .into_iter()
        .skip(calls)
        .find(|body| context_of(body).contains("MIDTURN-FIG"))
        .expect("a step read the mid-turn append");
    assert!(
        context_of(&after_append).contains(AGENT_PROMPT),
        "the step after the append runs as {AGENT}"
    );
    let calls = model.agent_requests().len();
    let sent = rig.agent(
        "alice",
        "s-alice",
        &["send", "--seat", &seat, "--nudge", "--body", "WAKE-PLUM"],
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    let woken = wait_for("the wake turn as the agent", Duration::from_secs(60), || {
        model
            .agent_requests()
            .iter()
            .skip(calls)
            .find(|body| last_text(body).starts_with(WAKE_UP))
            .cloned()
            .ok_or_else(|| read(&ledger_file))
    });
    assert!(context_of(&woken).contains("WAKE-PLUM"));
    assert!(
        context_of(&woken).contains(AGENT_PROMPT),
        "the plugin-started wake turn runs as {AGENT}"
    );
    wait_for("the agent's wake turn finished", Duration::from_secs(60), || {
        if opencode.all_idle() {
            Ok(())
        } else {
            Err("busy".to_owned())
        }
    });
    // The stored messages say the same: the append, the wake prompt and every step after them
    // carry the agent.
    let stored = opencode.messages(&session);
    let text_of_parts = |m: &Value| {
        m["parts"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<String>()
            })
            .unwrap_or_default()
    };
    let user_with = |needle: &str| {
        stored
            .iter()
            .find(|m| m["info"]["role"] == "user" && text_of_parts(m).contains(needle))
            .unwrap_or_else(|| panic!("a stored user message with {needle}"))
    };
    for needle in ["MIDTURN-FIG", WAKE_UP, "WAKE-PLUM"] {
        let message = if needle == WAKE_UP {
            stored
                .iter()
                .rev()
                .find(|m| m["info"]["role"] == "user" && text_of_parts(m).starts_with(WAKE_UP))
                .expect("the stored wake prompt")
        } else {
            user_with(needle)
        };
        assert_eq!(message["info"]["agent"], AGENT, "{needle}: {}", message["info"]);
        let id = message["info"]["id"].as_str().unwrap();
        for step in stored
            .iter()
            .filter(|m| m["info"]["role"] == "assistant" && m["info"]["parentID"] == id)
        {
            assert_eq!(step["info"]["agent"], AGENT, "{needle} step: {}", step["info"]);
        }
    }
    println!("evidence: {AGENT} kept on the mid-turn step and the wake turn");

    // 6. A subagent (task tool): its shell call claims a read-only child, its write is refused.
    let calls = model.agent_requests().len();
    opencode.prompt(
        &session,
        &format!(
            "RUNTASK: env | grep '^HERDR_THREADS_OPENCODE_PARENT'; herdr-threads send --seat {alice} --body CHILD-WRITE"
        ),
    );
    let child_requests: Vec<Value> = model.agent_requests().into_iter().skip(calls).collect();
    let child_first = child_requests
        .iter()
        .find(|body| last_text(body).starts_with("RUNBASH:"))
        .expect("the child session's first model call");
    println!("evidence: child first message:\n{}", last_text(child_first));
    let child_tool = child_requests
        .iter()
        .find_map(|body| {
            let last = body["messages"].as_array()?.last()?;
            let text = text_of(last);
            (last["role"] == "tool" && text.contains("HERDR_THREADS_OPENCODE_PARENT="))
                .then_some(text)
        })
        .expect("the child's bash tool result");
    println!("evidence: child bash result:\n{child_tool}");
    assert!(child_tool.contains(&format!("HERDR_THREADS_OPENCODE_PARENT={session}")));
    // The CLI refuses a subagent's write before it reaches the daemon (or the daemon does,
    // `subagent_read_only`); either way nothing is sent.
    assert!(
        child_tool.contains("only top-level agents") || child_tool.contains("subagent_read_only"),
        "{child_tool}"
    );
    assert!(
        last_text(child_first).contains("Subagents may discover"),
        "the child's first message carries the child restriction"
    );
    let alice_inbox = rig.agent("alice", "s-alice", &["inbox", "--machine"]);
    assert!(
        !alice_inbox.stdout.contains("CHILD-WRITE"),
        "{alice_inbox:?}"
    );

    // Stop opencode (the plugin's watch child goes with its process group), then unsetup.
    drop(opencode);
    let unsetup = {
        let mut command = rig.base(BIN);
        opencode_env(&mut command);
        command
            .args(["unsetup", "opencode", "--state-dir"])
            .arg(&rig.state)
            .arg("--json");
        run(command, None)
    };
    assert_eq!(unsetup.code, Some(0), "{unsetup:?}");
    assert!(!config_dir.join("plugins/herdr-threads.js").exists());
    let total = model.requests.lock().unwrap().len();
    println!(
        "evidence: model requests {total} (agent {}); ledger:\n{}",
        model.agent_requests().len(),
        read(&ledger_file)
    );
}
