//! ht-p03.44: wire compatibility of this run's optional additions across the
//! CLI/daemon seam, through the built executable and its detached daemon.
//!
//! A recording proxy takes the daemon's socket pathname (the daemon moves to a
//! sibling socket; the descriptor is re-pointed at the proxy's inode) so every
//! request the CLI sends is observed and a daemon vintage can be simulated:
//! `Old` closes on any request the pre-change daemon could not decode
//! (`Command::Capabilities`, `Command::HookParseFailure`, or a `full_bodies`
//! field under its deny_unknown_fields) and forwards the rest unchanged.
//! "Old CLI" frames are frozen under `tests/fixtures/old_wire/`; they are
//! protocol 1, two protocols behind this protocol-3 build (B5 moved to 2, thread
//! summaries to 3), so they exercise the decodable skew reply.

use super::sweep::{FakeHost, Scratch, pane};
use herdr_threads::test_support::spawn::SpawnOwned;
use herdr_threads::{
    daemon::{
        logs::daemon_log_path,
        ownership::{read_descriptor, read_existing_namespace},
        paths::{InstancePaths, RuntimeContext},
        remedy::{RemedyContext, remedy},
    },
    protocol::{results::ErrorClass, wire::PROTOCOL_VERSION},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const CURRENT: u8 = 0;
const OLD: u8 = 1;

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/old_wire")
            .join(name),
    )
    .unwrap()
}

fn read_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut prefix = [0_u8; 4];
    stream.read_exact(&mut prefix).ok()?;
    let mut body = vec![0_u8; u32::from_be_bytes(prefix) as usize];
    stream.read_exact(&mut body).ok()?;
    Some(body)
}

fn write_frame(stream: &mut UnixStream, body: &[u8]) -> std::io::Result<()> {
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(body)
}

/// One request the proxy saw.
#[derive(Clone, Debug)]
struct Seen {
    kind: String,
    /// The `full_bodies` key was present on the wire (any value).
    full_bodies_field: bool,
}

/// The recording proxy at the daemon's socket pathname.
struct Proxy {
    mode: Arc<AtomicU8>,
    log: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    socket: PathBuf,
    real: PathBuf,
    descriptor_path: PathBuf,
    original_descriptor: Vec<u8>,
}
impl Proxy {
    fn interpose(paths: &InstancePaths, root: &Path) -> Self {
        let instance = read_existing_namespace(paths).unwrap().unwrap();
        read_descriptor(paths, instance).expect("a published daemon");
        let socket = paths.socket_path.clone();
        let real = root.join("real.sock");
        let descriptor_path = paths.descriptor_path.clone();
        let original_descriptor = fs::read(&descriptor_path).unwrap();
        fs::rename(&socket, &real).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let meta = fs::metadata(&socket).unwrap();
        let mut descriptor: Value = serde_json::from_slice(&original_descriptor).unwrap();
        descriptor["socket_device"] = json!(meta.dev());
        descriptor["socket_inode"] = json!(meta.ino());
        write_private(&descriptor_path, &serde_json::to_vec(&descriptor).unwrap());

        let mode = Arc::new(AtomicU8::new(CURRENT));
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (mode_c, log_c, stop_c, real_c) =
            (mode.clone(), log.clone(), stop.clone(), real.clone());
        let worker = std::thread::spawn(move || {
            while !stop_c.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (mode, log, real) = (mode_c.clone(), log_c.clone(), real_c.clone());
                        std::thread::spawn(move || serve(stream, &mode, &log, &real));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("proxy accept: {error}"),
                }
            }
        });
        Self {
            mode,
            log,
            stop,
            worker: Some(worker),
            socket,
            real,
            descriptor_path,
            original_descriptor,
        }
    }
    fn set_mode(&self, mode: u8) {
        self.mode.store(mode, Ordering::SeqCst);
    }
    fn clear(&self) {
        self.log.lock().unwrap().clear();
    }
    fn seen(&self) -> Vec<Seen> {
        self.log.lock().unwrap().clone()
    }
    fn count(&self, kind: &str) -> usize {
        self.seen().iter().filter(|seen| seen.kind == kind).count()
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        // Hand the pathname and descriptor back so `daemon stop` finds the real one.
        let _ = fs::remove_file(&self.socket);
        let _ = fs::rename(&self.real, &self.socket);
        write_private(&self.descriptor_path, &self.original_descriptor);
    }
}

fn write_private(path: &Path, bytes: &[u8]) {
    let temporary = path.with_extension("tmp-wire-compat");
    fs::write(&temporary, bytes).unwrap();
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).unwrap();
    fs::rename(&temporary, path).unwrap();
}

fn serve(mut client: UnixStream, mode: &AtomicU8, log: &Mutex<Vec<Seen>>, real: &Path) {
    client.set_nonblocking(false).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let Some(body) = read_frame(&mut client) else {
        return;
    };
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let kind = request["command"]["kind"]
        .as_str()
        .unwrap_or("?")
        .to_owned();
    let full_bodies_field = request["command"]["args"]
        .as_object()
        .is_some_and(|args| args.contains_key("full_bodies"));
    log.lock().unwrap().push(Seen {
        kind: kind.clone(),
        full_bodies_field,
    });
    let undecodable_by_old =
        matches!(kind.as_str(), "capabilities" | "hook_parse_failure") || full_bodies_field;
    if mode.load(Ordering::SeqCst) == OLD && undecodable_by_old {
        return; // the old decoder cannot parse it: it closes without a reply
    }
    let Ok(mut daemon) = UnixStream::connect(real) else {
        return;
    };
    daemon
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    if write_frame(&mut daemon, &body).is_err() {
        return;
    }
    if let Some(reply) = read_frame(&mut daemon) {
        let _ = write_frame(&mut client, &reply);
    }
}

struct Plugin {
    root: PathBuf,
    state: PathBuf,
    host: PathBuf,
}
impl Plugin {
    fn command(&self) -> Command {
        let mut command = crate::scrubbed_command(BIN);
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
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"))
            .env("CODEX_HOME", self.root.join("codex-home"))
            .env("NO_COLOR", "1")
            // No harness-manifest fetch from the daemon this starts: its
            // late "fetch failed" line would land in the daemon log a test
            // compares, and wire compatibility does not involve it.
            .env("HERDR_THREADS_OFFLINE", "1");
        command
    }
    fn raw(&self, agent: Option<(&str, &str, &str)>, args: &[&str]) -> Output {
        let mut command = self.command();
        if let Some((seat, target, harness)) = agent {
            command.args([
                "--cooperative-seat",
                seat,
                "--cooperative-target",
                target,
                "--cooperative-harness",
                harness,
                "--cooperative-role",
                "top-level",
            ]);
        }
        command.args(args).output().unwrap()
    }
    fn ok(&self, agent: Option<(&str, &str, &str)>, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let output = self.raw(agent, &all);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success(),
            "herdr-threads {args:?} failed: {}{stdout}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_str::<Value>(&stdout).unwrap()["result"].clone()
    }
    fn paths(&self) -> InstancePaths {
        let context =
            RuntimeContext::explicit(self.state.clone(), self.host.clone(), None).unwrap();
        InstancePaths::resolve(&context).unwrap()
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.command().args(["daemon", "stop"]).output();
    }
}

/// A daemon with a seeded thread: one short message and three whose bodies are
/// clipped in a history preview (longer than the 256-character snippet) but
/// small enough for a `Message` fetch to return whole.
struct World {
    _scratch: Scratch,
    _host: FakeHost,
    plugin: Plugin,
    thread: String,
}
impl World {
    fn new() -> Self {
        let root = PathBuf::from(format!(
            "/private/tmp/htwc-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let scratch = Scratch(root.clone());
        let socket = root.join("herdr.sock");
        let host = FakeHost::start(
            &socket,
            vec![
                pane_labeled("w1:p1", "term-a", "alice"),
                pane_labeled("w1:p2", "term-b", "hatter"),
            ],
        );
        let plugin = Plugin {
            root: root.clone(),
            state: root.join("state"),
            host: socket,
        };
        plugin.ok(None, &["daemon", "ensure"]);
        let resolve = |pane: &str| {
            plugin.ok(None, &["seat", "resolve", "--pane", pane])["data"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let (alice_seat, hatter_seat) = (resolve("w1:p1"), resolve("w1:p2"));
        let alice = (alice_seat.as_str(), "w1:p1", "claude");
        let hatter = (hatter_seat.as_str(), "w1:p2", "codex");
        for agent in [alice, hatter] {
            plugin.ok(
                Some(agent),
                &["check-in", "--lifecycle-event", "agent-start"],
            );
        }
        let thread =
            plugin.ok(Some(alice), &["thread", "create", "--topic", "wire compat"])["data"]
                .as_str()
                .unwrap()
                .to_owned();
        plugin.ok(Some(alice), &["invite", &thread, "--seat", &hatter_seat]);
        plugin.ok(Some(hatter), &["accept", &thread]);
        plugin.ok(
            Some(alice),
            &["send", &thread, "--body", "a short opening line"],
        );
        for (agent, word) in [(hatter, "tea"), (alice, "jam"), (hatter, "bat")] {
            let long = format!(
                "long {word} body: {}",
                format!("{word} and more {word}. ").repeat(30)
            );
            assert!(long.chars().count() > 256 && long.len() < 4000);
            plugin.ok(Some(agent), &["send", &thread, "--body", &long]);
        }
        Self {
            _scratch: scratch,
            _host: host,
            plugin,
            thread,
        }
    }
    fn read(&self) -> Output {
        self.plugin
            .raw(None, &["read", &self.thread, "--human", "--recent", "10"])
    }
}

fn pane_labeled(id: &str, terminal: &str, label: &str) -> Value {
    let mut value = pane(id, terminal);
    value["label"] = json!(label);
    value
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Kills: a CLI that asks for full bodies and then still fetches each preview
/// (more than one History call, or any Message call), or one that never asks.
#[test]
fn cli_against_this_daemon_sends_full_bodies_and_makes_one_history_call() {
    let world = World::new();
    let proxy = Proxy::interpose(&world.plugin.paths(), &world.plugin.root);
    let output = world.read();
    assert!(output.status.success(), "{}", text(&output.stderr));
    let transcript = text(&output.stdout);
    // The whole 30-sentence body is on screen (line wrapping aside), not a clipped preview.
    let flat = transcript.split_whitespace().collect::<Vec<_>>().join(" ");
    let whole = "tea and more tea. ".repeat(30);
    assert!(flat.contains(whole.trim_end()), "{transcript}");
    let seen = proxy.seen();
    let kinds: Vec<&str> = seen.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(proxy.count("history"), 1, "{kinds:?}");
    assert_eq!(proxy.count("message"), 0, "no per-preview fetch: {kinds:?}");
    assert_eq!(proxy.count("capabilities"), 1, "{kinds:?}");
    assert!(
        seen.iter()
            .any(|s| s.kind == "history" && s.full_bodies_field),
        "the capable daemon was asked for full bodies: {kinds:?}"
    );
}

/// Kills: sending `full_bodies` to a daemon that did not advertise it (its
/// deny_unknown_fields decoder would close), skipping the per-preview fetch
/// fallback, or the two paths rendering different bytes.
#[test]
fn cli_against_an_old_fixture_daemon_never_sends_full_bodies() {
    let world = World::new();
    let proxy = Proxy::interpose(&world.plugin.paths(), &world.plugin.root);
    let current = world.read();
    assert!(current.status.success(), "{}", text(&current.stderr));
    assert_eq!(proxy.count("message"), 0);

    proxy.clear();
    proxy.set_mode(OLD);
    let old = world.read();
    assert!(old.status.success(), "{}", text(&old.stderr));
    let seen = proxy.seen();
    let kinds: Vec<&str> = seen.iter().map(|s| s.kind.as_str()).collect();
    assert!(seen.iter().all(|s| !s.full_bodies_field), "{kinds:?}");
    assert_eq!(proxy.count("history"), 1, "{kinds:?}");
    assert_eq!(
        proxy.count("message"),
        3,
        "one fetch per clipped preview: {kinds:?}"
    );
    assert_eq!(
        text(&old.stdout),
        text(&current.stdout),
        "identical rendered bytes"
    );
    assert_eq!(old.stderr, current.stderr);
}

/// Kills: a skewed daemon descriptor decoded or probed (a capability request,
/// a timeout, a deny_unknown_fields error) before the version is compared, and
/// a skew report whose remedy is not the stop-then-ensure pair that works.
#[test]
fn skewed_protocol_version_still_yields_the_stop_then_ensure_report() {
    let world = World::new();
    let paths = world.plugin.paths();
    let proxy = Proxy::interpose(&paths, &world.plugin.root);
    let published: Value =
        serde_json::from_slice(&fs::read(&paths.descriptor_path).unwrap()).unwrap();
    let software = published["software_version"].as_str().unwrap().to_owned();
    let mut skewed = published.clone();
    // The real release skew pair: a protocol-2 daemon (the last release, B5's
    // ht-rzi.23) against this protocol-3 CLI (thread summaries, ht-1ip).
    assert_eq!(PROTOCOL_VERSION, 3);
    skewed["protocol_version"] = json!(PROTOCOL_VERSION - 1);
    write_private(
        &paths.descriptor_path,
        &serde_json::to_vec(&skewed).unwrap(),
    );

    let started = Instant::now();
    let output = world.read();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "{stderr}");
    let expected = remedy(
        Some(ErrorClass::VersionSkew),
        &RemedyContext::VersionSkew {
            daemon: format!("{software} (protocol {})", PROTOCOL_VERSION - 1),
            cli: format!(
                "{} (protocol {PROTOCOL_VERSION})",
                env!("CARGO_PKG_VERSION")
            ),
        },
    );
    assert!(stderr.contains(&expected), "{stderr}");
    for banned in ["unknown field", "unknown variant", "decode", "timed out"] {
        assert!(!stderr.contains(banned), "{stderr}");
    }
    assert!(
        expected.contains("`herdr-threads daemon stop`"),
        "{expected}"
    );
    assert!(
        expected.contains("`herdr-threads daemon ensure`"),
        "{expected}"
    );
    assert!(
        proxy.seen().is_empty(),
        "skew is reported before any wire request: {:?}",
        proxy.seen()
    );

    // The remedy works: stop (skew tolerant), then ensure starts this version.
    drop(proxy);
    let stop = world.plugin.raw(None, &["daemon", "stop"]);
    assert!(
        stop.status.success(),
        "{}{}",
        text(&stop.stdout),
        text(&stop.stderr)
    );
    let ensured = world.plugin.ok(None, &["daemon", "ensure"]);
    assert_eq!(ensured["data"]["protocol_version"], PROTOCOL_VERSION);
}

/// Kills: a protocol-1 CLI (the frozen pre-B5 frames, two protocols behind
/// this protocol-3 daemon) that gets a closed socket or a reply
/// it cannot decode (a key outside its deny_unknown_fields types) instead of
/// the decodable skew error naming the stop-then-ensure remedy; and a skewed
/// request that reaches the service.
#[test]
fn old_protocol1_cli_gets_decodable_skew_replies() {
    let world = World::new();
    let paths = world.plugin.paths();
    let instance = read_existing_namespace(&paths).unwrap().unwrap();
    let descriptor = read_descriptor(&paths, instance).unwrap();
    assert_eq!(descriptor.protocol_version, PROTOCOL_VERSION);
    assert_ne!(
        PROTOCOL_VERSION, 1,
        "the frozen frames carry version 1, an older protocol than this build's"
    );
    let shapes: Value = serde_json::from_str(&fixture("reply_shapes.json")).unwrap();
    let allowed = |name: &str| -> Vec<String> {
        shapes[name]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key.as_str().unwrap().to_owned())
            .collect()
    };
    let exchange = |frame: &str| -> Value {
        let body = frame
            .replace("__INSTANCE__", &descriptor.instance_uuid.to_string())
            .replace("__THREAD__", &world.thread);
        let mut stream = UnixStream::connect(&descriptor.endpoint).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write_frame(&mut stream, body.trim().as_bytes()).unwrap();
        let reply = read_frame(&mut stream).expect("a reply, not a close");
        serde_json::from_slice(&reply).unwrap()
    };
    let only_known = |value: &Value, name: &str| {
        let allowed = allowed(name);
        for key in value.as_object().unwrap().keys() {
            assert!(
                allowed.contains(key),
                "{name} reply has key {key:?} an old CLI rejects"
            );
        }
    };
    // The protocol-1 ApiError fields (deny_unknown_fields at cf8c83d7).
    let error_keys = ["code", "detail", "restart_argv", "required_minimum_bytes"];

    for (fixture_name, request_id) in [
        ("health_request.json", "old-cli-health"),
        ("history_request.json", "old-cli-history"),
    ] {
        let reply = exchange(&fixture(fixture_name));
        only_known(&reply, "response");
        assert_eq!(reply["version"], 1, "echoes the sender's version: {reply}");
        assert_eq!(reply["request_id"], request_id);
        assert_eq!(reply["instance"], descriptor.instance_uuid.to_string());
        let error = &reply["result"]["Err"];
        for key in error.as_object().expect("a skew error").keys() {
            assert!(error_keys.contains(&key.as_str()), "error key {key:?}");
        }
        assert_eq!(error["code"], "unknown_wire_version", "{reply}");
        let detail = error["detail"].as_str().unwrap();
        assert!(detail.contains("daemon stop"), "{detail}");
        assert!(detail.contains("daemon ensure"), "{detail}");
    }
}

/// Kills: a hook that reports to a daemon that did not advertise
/// `hook.parse_failure_report` (the report is undecodable there), a hook that
/// waits out its budget on the refusal, or a decode error logged on either
/// side. The control run proves the report is observable when advertised.
#[test]
fn hook_parse_failure_under_optimistic_sends_no_report_to_an_old_daemon() {
    let world = World::new();
    let paths = world.plugin.paths();
    let proxy = Proxy::interpose(&paths, &world.plugin.root);
    let bin_dir = world.plugin.root.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let claude = bin_dir.join("claude");
    fs::write(&claude, "#!/bin/sh\necho '2.1.299 (Claude Code)'\n").unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    let run_hook = || {
        let mut command = crate::scrubbed_command(BIN);
        command
            .arg("--state-dir")
            .arg(&world.plugin.state)
            .arg("--host-endpoint")
            .arg(&world.plugin.host)
            .args(["hook", "claude"])
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", "w1:p1")
            .env("HERDR_SOCKET_PATH", &world.plugin.host)
            .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
            .env("HOME", world.plugin.root.join("home"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let started = Instant::now();
        let mut child = command.spawn_owned().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"unexpected":"payload shape"}"#)
            .unwrap();
        let output = child.wait_with_output().unwrap();
        (output, started.elapsed())
    };
    let log_path = daemon_log_path(&paths);
    let daemon_log = || fs::read_to_string(&log_path).unwrap_or_default();

    proxy.set_mode(OLD);
    let log_before = daemon_log();
    assert!(
        log_before.contains("harness manifest: off (HERDR_THREADS_OFFLINE=1)"),
        "no manifest fetch can append to the compared log: {log_before}"
    );
    let (output, elapsed) = run_hook();
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert!(output.stdout.is_empty(), "{}", text(&output.stdout));
    assert!(
        stderr.contains("unsupported hook payload"),
        "the payload really failed to parse: {stderr}"
    );
    assert!(
        elapsed < herdr_threads::cli::hook::TOOL_BUDGET,
        "within the normal hook time: {elapsed:?}"
    );
    let kinds: Vec<String> = proxy.seen().into_iter().map(|s| s.kind).collect();
    assert!(
        !kinds.iter().any(|kind| kind == "hook_parse_failure"),
        "{kinds:?}"
    );
    assert!(!stderr.contains("budget expired"), "{stderr}");
    for banned in ["decode", "unknown variant", "unknown field"] {
        assert!(!stderr.contains(banned), "hook stderr: {stderr}");
        assert!(
            !daemon_log().to_lowercase().contains(banned),
            "daemon log: {}",
            daemon_log()
        );
    }
    assert_eq!(
        daemon_log(),
        log_before,
        "the old-daemon hook left the daemon log untouched"
    );

    // Control: the same hook against an advertising daemon does send one report.
    proxy.clear();
    proxy.set_mode(CURRENT);
    let (output, _) = run_hook();
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(proxy.count("hook_parse_failure"), 1, "{:?}", proxy.seen());
}
