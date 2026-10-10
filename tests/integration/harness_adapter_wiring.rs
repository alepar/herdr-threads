//! Registry-to-executable-to-daemon conservation net. All callback/runtime
//! values and the protocol host are synthetic; none is native Hermes proof.
use super::sweep::{FakeHost, pane};
use herdr_threads::{
    daemon::{
        ownership::{read_descriptor, read_existing_namespace},
        paths::{InstancePaths, RuntimeContext},
    },
    test_support::{isolation::TestIsolation, spawn::SpawnOwned},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const CURRENT: u8 = 0;
const ABSENT: u8 = 1;
const MALFORMED_FALSE: u8 = 2;
const WRONG_RESULT: u8 = 3;
const REFUSED: u8 = 4;
const MAX_FRAME: usize = 1_048_576;

struct World {
    isolation: TestIsolation,
    _host: FakeHost,
    state: PathBuf,
    endpoint: PathBuf,
    fourth: PathBuf,
}
impl World {
    fn new() -> Self {
        let isolation = TestIsolation::new("adapter-wiring");
        let state = isolation.path("state");
        let endpoint = isolation.socket_path("host.sock");
        let fourth = isolation.home().join("fourth profile");
        fs::create_dir_all(&fourth).unwrap();
        let binary = fourth.join("ht-synthetic-fourth");
        fs::write(&binary, b"synthetic fourth fixture\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let paths = InstancePaths::resolve(
            &RuntimeContext::explicit(state.clone(), endpoint.clone(), None).unwrap(),
        )
        .unwrap();
        paths.prepare_instance_dir().unwrap();
        let settings = paths.instance_dir.join("settings.json");
        // Use the actual settings filename, not an ineffective environment hint.
        let settings = settings.with_file_name(herdr_threads::daemon::settings::SETTINGS_FILE);
        fs::write(&settings, br#"{"harness_manifest":"off"}"#).unwrap();
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
        let host = FakeHost::start(
            &endpoint,
            vec![pane("w1:p1", "fourth-term"), pane("w1:p2", "hermes-term")],
        );
        let selected_parent = isolation.socket_path("selected");
        std::os::unix::fs::symlink(endpoint.parent().unwrap(), &selected_parent).unwrap();
        let selected_endpoint = selected_parent.join("host.sock");
        let world = Self {
            isolation,
            _host: host,
            state,
            endpoint: selected_endpoint,
            fourth,
        };
        world.json(&["daemon", "ensure"]);
        world
    }
    fn paths(&self) -> InstancePaths {
        InstancePaths::resolve(
            &RuntimeContext::explicit(self.state.clone(), self.endpoint.clone(), None).unwrap(),
        )
        .unwrap()
    }
    fn command(&self) -> Command {
        let mut c = self.isolation.command(BIN);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HERMES_") {
                c.env_remove(key);
            }
        }
        c.env("PATH", format!("{}:/usr/bin:/bin", self.fourth.display()))
            .env("HT_SYNTHETIC_FOURTH_ROOT", &self.fourth)
            .env("CLAUDE_CONFIG_DIR", self.isolation.path("home/claude"))
            .env("CODEX_HOME", self.isolation.path("home/codex"))
            .env("HERMES_HOME", self.isolation.path("home/hermes"))
            .env("HERDR_THREADS_OFFLINE", "1");
        c
    }
    fn routed(&self, args: &[&str]) -> Vec<std::ffi::OsString> {
        herdr_threads::test_support::isolation::routed_argv(
            &[
                std::ffi::OsStr::new("--state-dir"),
                self.state.as_os_str(),
                std::ffi::OsStr::new("--host-endpoint"),
                self.endpoint.as_os_str(),
            ],
            args,
        )
    }
    fn raw(&self, args: &[&str]) -> Output {
        self.command()
            .args(self.routed(args))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap()
            .wait_with_output()
            .unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let mut words = vec!["--json"];
        words.extend_from_slice(args);
        let output = self.raw(&words);
        assert!(
            output.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn hook(&self, harness: &str, pane: &str, payload: &Value) -> Value {
        let mut child = self
            .command()
            .args(self.routed(&["hook", harness]))
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", pane)
            .env("HERDR_SOCKET_PATH", &self.endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(payload).unwrap())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "hook {harness}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if output.stdout.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&output.stdout).unwrap()
        }
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.paths().database_path).unwrap()
    }
    fn protected(&self) -> Vec<Vec<String>> {
        let db = self.db();
        [
            "seats",
            "occupant_bindings",
            "operations",
            "receipts",
            "invitations",
            "warning_offer",
            "digest_notice_offer",
            "receipt_state",
            "delivery_observations",
        ]
        .map(|table| {
            let mut s = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let n = s.column_count();
            let mut rows = s
                .query_map([], |r| {
                    Ok(format!(
                        "{:?}",
                        (0..n)
                            .map(|i| r.get::<_, rusqlite::types::Value>(i))
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort();
            rows
        })
        .into()
    }
    fn binding(&self) -> (String, i64, String, String, String) {
        self.db().query_row("SELECT seat_id,generation,native_session,execution_id,observation_provenance FROM occupant_bindings WHERE harness='hermes' AND ended_at IS NULL", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap()
    }
}
impl Drop for World {
    fn drop(&mut self) {
        let output = self.raw(&["daemon", "stop"]);
        if !std::thread::panicking() {
            assert!(output.status.success(), "owned daemon stop failed");
        }
    }
}
fn envelope(session: &str, event: &str, sequence: u64) -> Value {
    let mut p: Value =
        serde_json::from_slice(include_bytes!("../fixtures/hermes/envelopes.json")).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    p["session_id"] = session.into();
    p["event_id"] = event.into();
    p["turn_id"] = event.into();
    p["role_association"]["session_id"] = session.into();
    p["role_association"]["turn_id"] = event.into();
    p["started_at"] = now.into();
    p["deadline_at"] = (now + 1200).into();
    p["observation_order"]["observed_at_millis"] = now.into();
    p["observation_order"]["sequence"] = sequence.into();
    p
}
fn post(session: &str, turn: &str, event: &str, sequence: u64) -> Value {
    let mut p = envelope(session, event, sequence);
    p["callback"] = "post_tool_call".into();
    p["turn_id"] = turn.into();
    p["parent_session_id"] = Value::Null;
    p["shape"]["parent_session_id"] = json!({"presence":"missing","type":"absent"});
    p["role_association"]["turn_id"] = turn.into();
    p["role_association"]["provenance"] = "qualified_pre_llm_cache".into();
    p
}
fn remaining(at: Instant) -> io::Result<Duration> {
    at.checked_duration_since(Instant::now())
        .map(|d| Duration::from_millis(d.as_millis() as u64))
        .filter(|d| !d.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "fixture transaction deadline"))
}
fn read_exact_until(
    stream: &mut UnixStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> io::Result<()> {
    stream.set_nonblocking(true)?;
    while !bytes.is_empty() {
        remaining(deadline)?;
        match stream.read(bytes) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => bytes = &mut bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(remaining(deadline)?.min(Duration::from_millis(1)))
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn write_until(stream: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    stream.set_nonblocking(true)?;
    while !bytes.is_empty() {
        remaining(deadline)?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(remaining(deadline)?.min(Duration::from_millis(1)))
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn read_frame(stream: &mut UnixStream, deadline: Instant) -> io::Result<Vec<u8>> {
    let mut prefix = [0; 4];
    read_exact_until(stream, &mut prefix, deadline)?;
    let n = u32::from_be_bytes(prefix) as usize;
    if n > MAX_FRAME {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut body = vec![0; n];
    read_exact_until(stream, &mut body, deadline)?;
    Ok(body)
}
fn write_frame(stream: &mut UnixStream, body: &[u8], deadline: Instant) -> io::Result<()> {
    if body.len() > MAX_FRAME {
        return Err(io::ErrorKind::InvalidData.into());
    }
    write_until(stream, &(body.len() as u32).to_be_bytes(), deadline)?;
    write_until(stream, body, deadline)
}
/// Only owned test sockets are touched. One worker/transaction, no queue or
/// detached workers. Partial IO shares one absolute deadline.
struct Proxy {
    mode: Arc<AtomicU8>,
    seen: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    active: Arc<Mutex<Vec<UnixStream>>>,
    worker: Option<JoinHandle<()>>,
    socket: PathBuf,
    real: PathBuf,
    descriptor: PathBuf,
    original: Vec<u8>,
    interposed: bool,
}
impl Proxy {
    fn new(w: &World) -> Self {
        let paths = w.paths();
        let ns = read_existing_namespace(&paths).unwrap().unwrap();
        read_descriptor(&paths, ns).unwrap();
        let original = fs::read(&paths.descriptor_path).unwrap();
        let real = w.isolation.socket_path("upstream.sock");
        let mode = Arc::new(AtomicU8::new(CURRENT));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(Mutex::new(Vec::new()));
        let mut owned = Self {
            mode: mode.clone(),
            seen: seen.clone(),
            stop: stop.clone(),
            active: active.clone(),
            worker: None,
            socket: paths.socket_path.clone(),
            real: real.clone(),
            descriptor: paths.descriptor_path.clone(),
            original: original.clone(),
            interposed: false,
        };
        fs::rename(&paths.socket_path, &real).unwrap();
        owned.interposed = true;
        let listener = UnixListener::bind(&paths.socket_path).unwrap();
        fs::set_permissions(&paths.socket_path, fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let m = fs::metadata(&paths.socket_path).unwrap();
        let mut descriptor: Value = serde_json::from_slice(&original).unwrap();
        descriptor["socket_device"] = m.dev().into();
        descriptor["socket_inode"] = m.ino().into();
        fs::write(
            &paths.descriptor_path,
            serde_json::to_vec(&descriptor).unwrap(),
        )
        .unwrap();
        let (mode_c, seen_c, stop_c, active_c, real_c) = (
            mode.clone(),
            seen.clone(),
            stop.clone(),
            active.clone(),
            real.clone(),
        );
        let worker = std::thread::spawn(move || {
            while !stop_c.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut client, _)) => {
                        let _ = client.set_nonblocking(false);
                        let deadline = Instant::now() + Duration::from_secs(2);
                        {
                            let mut sockets = active_c.lock().unwrap();
                            sockets.clear();
                            sockets.push(client.try_clone().unwrap());
                        }
                        let served = (|| -> io::Result<()> {
                            let mut upstream = UnixStream::connect(&real_c).map_err(|e| {
                                io::Error::new(e.kind(), format!("upstream connect: {e}"))
                            })?;
                            active_c.lock().unwrap().push(upstream.try_clone()?);
                            for _ in 0..16 {
                                let frame = read_frame(&mut client, deadline)?;
                                let request: Value =
                                    serde_json::from_slice(&frame).map_err(io::Error::other)?;
                                // Private source fixtures contain no native body; keep only command, not full envelope.
                                let kind = request["command"]["kind"]
                                    .as_str()
                                    .unwrap_or("?")
                                    .to_owned();
                                seen_c.lock().unwrap().push(request["command"].clone());
                                if stop_c.load(Ordering::SeqCst) {
                                    return Ok(());
                                }
                                write_frame(&mut upstream, &frame, deadline)?;
                                let reply = read_frame(&mut upstream, deadline)?;
                                let mut reply: Value =
                                    serde_json::from_slice(&reply).map_err(io::Error::other)?;
                                match (mode_c.load(Ordering::SeqCst), kind.as_str()) {
                                    (ABSENT, "capabilities") => {
                                        if let Some(list) = reply
                                            .pointer_mut("/result/Ok/data/capabilities")
                                            .and_then(Value::as_array_mut)
                                        {
                                            list.retain(|v| {
                                                v != "hook.harness_evidence_v2"
                                                    && v != "harness.health_v2"
                                            });
                                        }
                                    }
                                    (MALFORMED_FALSE, "capabilities") => {
                                        reply["result"]["Ok"]["data"]["capabilities"] =
                                            json!([false])
                                    }
                                    (WRONG_RESULT, "harness_evidence_v2" | "harness_health_v2") => {
                                        reply["result"]["Ok"] = json!({"kind":"capabilities","data":{"capabilities":[]}})
                                    }
                                    (REFUSED, "harness_evidence_v2" | "harness_health_v2") => {
                                        reply["result"] = json!({"Err":{"code":"unsupported","detail":"owned fixture refusal"}})
                                    }
                                    _ => {}
                                }
                                write_frame(
                                    &mut client,
                                    &serde_json::to_vec(&reply).map_err(io::Error::other)?,
                                    deadline,
                                )?;
                            }
                            Ok(())
                        })();
                        if let Err(ref e) = served
                            && e.kind() != io::ErrorKind::UnexpectedEof
                        {
                            eprintln!("proxy fixture: {e}");
                        }
                        active_c.lock().unwrap().clear();
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("owned proxy accept: {e}"),
                }
            }
        });
        owned.worker = Some(worker);
        owned
    }
    fn set(&self, mode: u8) {
        self.mode.store(mode, Ordering::SeqCst);
        self.seen.lock().unwrap().clear();
    }
    fn seen(&self) -> Vec<Value> {
        self.seen.lock().unwrap().clone()
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for stream in self.active.lock().unwrap().iter() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        let _ = UnixStream::connect(&self.socket);
        let joined = self.worker.take().map_or(Ok(()), |worker| worker.join());
        if !self.interposed {
            return;
        }
        let removed = match fs::remove_file(&self.socket) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
        let restored = removed
            .and_then(|()| fs::rename(&self.real, &self.socket))
            .and_then(|()| fs::write(&self.descriptor, &self.original));
        if !std::thread::panicking() {
            assert!(joined.is_ok(), "owned proxy did not join");
            assert!(restored.is_ok(), "owned socket restoration failed");
        }
    }
}

#[test]
fn harness_adapter_end_to_end_wiring_uses_registry_values_across_boundaries() {
    let w = World::new();
    assert!(
        fs::symlink_metadata(w.endpoint.parent().unwrap())
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_ne!(w.endpoint, w.endpoint.canonicalize().unwrap());
    let discovery = w.json(&["adapters"]);
    assert_eq!(
        discovery["adapters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["claude", "codex", "hermes", "synthetic_fourth"]
    );
    let binary = w.fourth.join("ht-synthetic-fourth");
    w.json(&[
        "setup",
        "synthetic_fourth",
        "--harness-binary",
        binary.to_str().unwrap(),
    ]);
    let status = w.json(&["setup-status", "synthetic_fourth"]);
    assert!(
        status["setup"]["installed"] == true,
        "actual fourth setup/status: {status}"
    );
    let seat = w.json(&["seat", "resolve", "--pane", "w1:p1"])["result"]["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let fourth = json!({"event":"SyntheticStart","event_id":"fourth-start","session_id":"fourth-session","role":"top_level"});
    assert!(w.hook("synthetic_fourth", "w1:p1", &fourth)["synthetic_context"].is_string());
    let actual:(String,String)=w.db().query_row("SELECT harness,observation_provenance FROM occupant_bindings WHERE seat_id=? AND ended_at IS NULL",[&seat],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        actual,
        ("synthetic_fourth".into(), "cooperative_top_level".into())
    );
    w.json(&["seat", "resolve", "--pane", "w1:p2"]);
    let proxy = Proxy::new(&w);
    let start = w.hook("hermes", "w1:p2", &envelope("net-session", "net-start", 1));
    assert_eq!(
        start["lifecycle_ack"]["mode"],
        "startup",
        "output={start} seen={:?}",
        proxy.seen()
    );
    let initial = w.binding();
    assert_eq!(initial.4, "cooperative_top_level");
    let current = w.hook(
        "hermes",
        "w1:p2",
        &envelope("net-session", "net-current", 2),
    );
    assert_eq!(current["lifecycle_ack"]["mode"], "current");
    assert_eq!(w.binding(), initial);
    assert_eq!(
        w.hook(
            "hermes",
            "w1:p2",
            &envelope("net-session", "net-current", 2)
        ),
        current
    );
    let before = w.protected();
    assert!(
        w.hook(
            "hermes",
            "w1:p2",
            &post("net-session", "net-current", "net-post", 3)
        )["lifecycle_ack"]
            .is_null()
    );
    assert_eq!(w.protected(), before, "observer mutated authority");
    let notes = proxy
        .seen()
        .into_iter()
        .filter(|v| v["kind"] == "harness_evidence_v2")
        .collect::<Vec<_>>();
    for domain in ["native_callback", "bridge_envelope"] {
        let declaration = discovery["adapters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == "hermes")
            .unwrap()["contracts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["domain"] == domain)
            .unwrap();
        assert!(
            notes.iter().any(|n| n["args"]["domain"] == domain
                && n["args"]["contract_id"] == declaration["id"]
                && n["args"]["runtime"] == envelope("x", "y", 1)["runtime_identity"]),
            "actual domain frame missing {domain}: {notes:?}"
        );
        let (contract,milestones):(String,String)=w.db().query_row("SELECT contract_id,milestones_json FROM harness_contract_evidence_v2 WHERE harness='hermes' AND domain=?",[domain],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(contract, declaration["id"].as_str().unwrap());
        let milestones: Value = serde_json::from_str(&milestones).unwrap();
        assert!(milestones.get("qualified_turn").is_some());
        assert!(milestones.get("qualified_post_tool").is_some());
    }
    for (i, case) in ["child", "unknown", "stale", "invalid_identity"]
        .into_iter()
        .enumerate()
    {
        let mut rejected = envelope(
            "net-session",
            &format!("net-rejected-{case}"),
            30 + i as u64,
        );
        match case {
            "child" => {
                rejected["parent_session_id"] = "parent".into();
                rejected["role_association"]["role"] = "child".into();
            }
            "unknown" => {
                rejected["parent_session_id"] = Value::Null;
                rejected["role_association"]["role"] = "unknown".into();
                rejected["shape"]["parent_session_id"] =
                    json!({"presence":"missing","type":"absent"});
            }
            "stale" => rejected["timeout_observation"]["age_seconds"] = 6.0.into(),
            "invalid_identity" => rejected["runtime_identity"]["key"] = "build:invalid".into(),
            _ => unreachable!(),
        }
        assert!(
            w.hook("hermes", "w1:p2", &rejected)["lifecycle_ack"].is_null(),
            "{case} acquired authority"
        );
        assert_eq!(w.protected(), before, "{case} changed accountable rows");
    }
    let mut reset = envelope("net-new", "net-reset", 40);
    reset["callback"] = "on_session_reset".into();
    reset["reset_reason"] = "new_session".into();
    reset["role_association"] = Value::Null;
    assert!(w.hook("hermes", "w1:p2", &reset).is_null());
    assert_eq!(w.binding(), initial);
    let cleared = w.hook("hermes", "w1:p2", &envelope("net-new", "net-clear", 41));
    assert_eq!(cleared["lifecycle_ack"]["mode"], "clear");
    let updated = w.binding();
    assert_eq!(updated.0, initial.0);
    assert!(updated.1 > initial.1);
    assert_ne!(updated.3, initial.3);
    assert!(updated.2.contains("net-new"));
    drop(proxy);
    let doctor = w.json(&["doctor", "--harness", "synthetic_fourth"]);
    assert_eq!(
        doctor["doctor"]["adapter_order"],
        json!(["synthetic_fourth"])
    );
    assert_eq!(
        doctor["doctor"]["local_harnesses"]["synthetic_fourth"]["installed"], true,
        "doctor={doctor}"
    );
    assert_eq!(
        doctor["doctor"]["harness_health_v2"]["harnesses"]["hermes"]["scope"]["kind"],
        "daemon_default"
    );
    assert_eq!(
        doctor["doctor"]["local_harnesses"]["synthetic_fourth"]["scope"]["kind"],
        "local_config_root"
    );
    let rich = doctor["doctor"]["harness_health_v2"]["harnesses"]["hermes"]["runtime_evidence"]
        .as_array()
        .unwrap();
    assert_eq!(rich.len(), 2);
    assert!(rich.iter().all(|row| {
        row["scope"]["kind"] == "runtime_evidence_all_scopes"
            && row["source"]
                .as_str()
                .unwrap()
                .contains("model stage unclassified")
    }));
    w.json(&["unsetup", "synthetic_fourth"]);
    assert!(!w.fourth.join("owned-hooks.json").exists());
}

#[test]
fn hermes_observer_actual_wire_negotiation_never_downgrades_or_changes_authority() {
    let w = World::new();
    w.json(&["seat", "resolve", "--pane", "w1:p2"]);
    w.hook("hermes", "w1:p2", &envelope("neg-session", "neg-start", 1));
    let before = w.protected();
    let proxy = Proxy::new(&w);
    for (i, mode) in [ABSENT, MALFORMED_FALSE, WRONG_RESULT, REFUSED]
        .into_iter()
        .enumerate()
    {
        proxy.set(mode);
        w.hook(
            "hermes",
            "w1:p2",
            &post(
                "neg-session",
                "neg-start",
                &format!("neg-post-{i}"),
                10 + i as u64,
            ),
        );
        assert_eq!(
            w.protected(),
            before,
            "unsupported observer mutated accountable rows"
        );
        let seen = proxy.seen();
        assert!(
            seen.iter().any(|c| c["kind"] == "capabilities"),
            "negotiation absent: {seen:?}"
        );
        assert!(
            !seen.iter().any(|c| c["kind"] == "harness_evidence"),
            "rich evidence downgraded"
        );
        assert!(
            seen.iter().all(|c| matches!(
                c["kind"].as_str(),
                Some("capabilities" | "harness_evidence_v2")
            )),
            "observer submitted authority command: {seen:?}"
        );
        let rich = seen
            .iter()
            .filter(|c| c["kind"] == "harness_evidence_v2")
            .count();
        assert_eq!(
            rich,
            if mode == ABSENT || mode == MALFORMED_FALSE {
                0
            } else {
                2
            },
            "mode={mode} seen={seen:?}"
        );
    }
    proxy.set(CURRENT);
    w.hook(
        "hermes",
        "w1:p2",
        &post("neg-session", "neg-start", "neg-post-success", 20),
    );
    assert_eq!(
        proxy
            .seen()
            .iter()
            .filter(|c| c["kind"] == "harness_evidence_v2")
            .count(),
        2,
        "failed send suppressed required retry"
    );
    assert_eq!(w.protected(), before);
}

#[test]
fn owned_proxy_shutdown_joins_partial_peer_and_restores_daemon_endpoint() {
    let w = World::new();
    let proxy = Proxy::new(&w);
    let mut peer = UnixStream::connect(&proxy.socket).unwrap();
    peer.write_all(&100_u32.to_be_bytes()).unwrap();
    peer.write_all(b"{").unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while proxy.active.lock().unwrap().is_empty() {
        assert!(
            Instant::now() < deadline,
            "owned worker did not accept fixture"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let began = Instant::now();
    drop(proxy);
    eprintln!(
        "owned proxy partial-peer join completed in {:?}",
        began.elapsed()
    );
    drop(peer);
    assert_eq!(w.json(&["daemon", "health"])["result"]["kind"], "health");
}
