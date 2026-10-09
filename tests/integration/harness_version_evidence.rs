//! ht-xoc.7: harness version evidence end to end, with a stand-in harness.
//!
//! The test plays the harness: it writes a transcript JSONL whose entries carry
//! `"version"` and pipes hook payloads (built from the committed
//! `claude-2.1.286` captures with `transcript_path` and `session_id` rewritten)
//! into the built executable's `hook claude --event <E>`, against the real
//! detached daemon (`daemon ensure`) on an isolated state dir, HOME,
//! CLAUDE_CONFIG_DIR and a scratch PATH holding a stand-in `claude`. No model,
//! no real harness, no Herdr server (the daemon reports its host as down; the
//! assertions look only at the harness-version lines, never at overall health).
//!
//! Fake manifests are generated at test time by the canary writer
//! (`scripts/canary/manifest.py write|set`) against the built binary and served
//! to the daemon through `HT_TEST_MANIFEST_URL=file://...`. A scenario that
//! needs `python3` or `curl` prints why and skips when either is missing.
use herdr_threads::{harness::state::Ladder, test_support::spawn::SpawnOwned};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Sender, TryRecvError, channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
/// Unlisted and newer than anything the recipe tables verified.
const NEW_VERSION: &str = "2.1.999";
/// An unlisted version for the scenarios that need a second, distinct one.
const OTHER_VERSION: &str = "2.1.998";
/// A contract id the built binary does not have (a "release" with another contract).
const OTHER_CONTRACT: &str = "dddddddddddddddd";

fn tool_present(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Skip (with the reason printed) a scenario that needs the manifest writer
/// or curl; the daemon's fetcher is curl.
fn tools_or_skip(test: &str) -> bool {
    for tool in ["python3", "curl"] {
        if !tool_present(tool) {
            eprintln!("skipping {test}: `{tool}` is not on PATH");
            return false;
        }
    }
    true
}

fn wait_for<T>(what: &str, timeout: Duration, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Hold a local HTTP response until released. Dropping the server cancels
/// its bounded accept/read/channel waits and joins the helper on panic too.
struct GatedServer {
    url: String,
    accepted: Arc<AtomicUsize>,
    go: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl GatedServer {
    const GIVE_UP: Duration = Duration::from_secs(30);

    fn start(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let accepted = Arc::new(AtomicUsize::new(0));
        let (go, gate) = channel::<()>();
        let seen = Arc::clone(&accepted);
        let thread = std::thread::spawn(move || {
            let deadline = Instant::now() + Self::GIVE_UP;
            let stream = wait_for("the gated server's connection", Self::GIVE_UP, || {
                if matches!(gate.try_recv(), Err(TryRecvError::Disconnected)) {
                    return Some(None);
                }
                match listener.accept() {
                    Ok((stream, _)) => Some(Some(stream)),
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => None,
                    Err(_) => Some(None),
                }
            });
            let Some(mut stream) = stream else {
                return;
            };
            stream.set_nonblocking(false).unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 512];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() || matches!(gate.try_recv(), Err(TryRecvError::Disconnected))
                {
                    return;
                }
                stream
                    .set_read_timeout(Some(remaining.min(Duration::from_millis(100))))
                    .unwrap();
                match stream.read(&mut chunk) {
                    Ok(0) => return,
                    Ok(n) => request.extend_from_slice(&chunk[..n]),
                    Err(err)
                        if matches!(
                            err.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => return,
                }
            }
            seen.fetch_add(1, Ordering::SeqCst);
            if gate
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .is_err()
            {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            stream.set_write_timeout(Some(remaining)).unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"gated\"\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });
        Self {
            url,
            accepted,
            go: Some(go),
            thread: Some(thread),
        }
    }

    fn release_and_join(&mut self) {
        self.go.take().unwrap().send(()).unwrap();
        self.thread.take().unwrap().join().unwrap();
    }
}

impl Drop for GatedServer {
    fn drop(&mut self) {
        // Disconnect first, so failure before accept, during read, or while
        // held wakes the helper before we join it.
        self.go.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One isolated daemon instance plus the stand-in harness around it.
struct Rig {
    root: PathBuf,
    state: PathBuf,
    host: PathBuf,
    bin: PathBuf,
    daemon_env: Vec<(String, String)>,
}

/// One hook run: its exit code and stdout.
struct HookRun {
    code: Option<i32>,
    stdout: String,
}

impl Rig {
    fn new(version: Option<&str>) -> Self {
        let root = PathBuf::from(format!(
            "/private/tmp/ht-hve-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        for dir in ["", "st", "bin", "home", "work"] {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(root.join(dir))
                .unwrap();
        }
        let rig = Self {
            state: root.join("st"),
            host: root.join("herdr.sock"),
            bin: root.join("bin"),
            root,
            daemon_env: Vec::new(),
        };
        if let Some(version) = version {
            rig.stand_in(version);
        }
        rig
    }

    /// A `#!/bin/sh` stand-in `claude` printing `<version> (Claude Code)`.
    fn stand_in(&self, version: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.bin.join("claude");
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' '{version} (Claude Code)'\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Resolve real curl before installing this scenario's wrapper. Curl's
    /// last --max-time wins; exec preserves every other argument and output,
    /// with a finite fetch budget matching the gated server's lifetime.
    fn curl_with_test_timeout(&self) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let wrapper = self.bin.join("curl");
        assert!(
            !wrapper.exists(),
            "resolve curl before installing its wrapper"
        );
        let command = self.command(&[], &[]);
        let search = command
            .get_envs()
            .find_map(|(key, value)| (key == "PATH").then_some(value).flatten())
            .unwrap();
        let real = std::env::split_paths(search)
            .map(|dir| dir.join("curl"))
            .find(|path| {
                fs::metadata(path)
                    .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            })
            .expect("real curl is on the rig's PATH before the wrapper is installed");
        let quoted = real.to_str().unwrap().replace('\'', "'\"'\"'");
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexec '{quoted}' \"$@\" --max-time {}\n",
                GatedServer::GIVE_UP.as_secs()
            ),
        )
        .unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        wrapper
    }

    fn command(&self, args: &[&str], env: &[(String, String)]) -> Command {
        let mut command = crate::scrubbed_command(BIN);
        command
            .args(herdr_threads::test_support::isolation::routed_argv(
                &[
                    std::ffi::OsStr::new("--state-dir"),
                    AsRef::<std::ffi::OsStr>::as_ref(&&self.state),
                    std::ffi::OsStr::new("--host-endpoint"),
                    AsRef::<std::ffi::OsStr>::as_ref(&&self.host),
                ],
                args,
            ))
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"))
            .env("CODEX_HOME", self.root.join("codex-home"))
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH");
        command.envs(env.iter().map(|(k, v)| (k, v)));
        command
    }

    fn cli(&self, args: &[&str]) -> Output {
        self.command(args, &[]).output().unwrap()
    }

    /// Start the daemon with `env` added to its environment (the detached
    /// daemon inherits the `ensure` process's environment).
    fn start(&mut self, env: &[(&str, &str)]) {
        self.daemon_env = env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let output = self
            .command(&["daemon", "ensure"], &self.daemon_env)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "daemon ensure: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn manifest_url(path: &Path) -> String {
        format!("file://{}", path.display())
    }

    // ----- the stand-in harness

    /// A transcript whose entries carry `version`; returns its path.
    fn transcript(&self, session: &str, version: &str) -> PathBuf {
        let path = self.root.join("work").join(format!("{session}.jsonl"));
        let lines = [
            json!({"type": "user", "version": version, "sessionId": session}),
            json!({"type": "assistant", "version": version, "sessionId": session}),
        ];
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        fs::write(&path, text).unwrap();
        path
    }

    /// The committed 2.1.286 capture for `event`, with the transcript and
    /// session rewritten and `drop` top-level fields removed.
    fn payload(event: &str, transcript: &Path, session: &str, drop: &[&str]) -> Vec<u8> {
        let name = match event {
            "SessionStart" => "01-sessionstart-startup.json",
            "PreToolUse" => "02-pretooluse-bash-root.json",
            other => panic!("no capture for {other}"),
        };
        let mut value: Value = serde_json::from_slice(
            &fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/claude-2.1.286")
                    .join(name),
            )
            .unwrap(),
        )
        .unwrap();
        value["transcript_path"] = json!(transcript);
        value["session_id"] = json!(session);
        for field in drop {
            value.as_object_mut().unwrap().remove(*field);
        }
        serde_json::to_vec(&value).unwrap()
    }

    /// `herdr-threads hook claude [--event E]` with `stdin`, outside any Herdr
    /// pane unless `in_pane`. `scaled: false` runs with the production budgets.
    fn hook(&self, event: Option<&str>, stdin: &[u8], in_pane: bool, scaled: bool) -> HookRun {
        let mut args = vec!["hook", "claude"];
        if let Some(event) = event {
            args.extend(["--event", event]);
        }
        let mut command = self.command(&args, &[]);
        if !scaled {
            command.env_remove(herdr_threads::protocol::time::TEST_TIMEOUT_SCALE_ENV);
        }
        if in_pane {
            command
                .env("HERDR_ENV", "1")
                .env("HERDR_PANE_ID", "p_stand_in")
                .env("HERDR_SOCKET_PATH", &self.host);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        let output = child.wait_with_output().unwrap();
        HookRun {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        }
    }

    // ----- readers

    fn doctor_json(&self) -> Value {
        let output = self.cli(&["doctor", "--json"]);
        serde_json::from_slice::<Value>(&output.stdout).unwrap_or_else(|e| {
            panic!(
                "doctor --json: {e}: {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }

    fn doctor_text(&self) -> String {
        String::from_utf8_lossy(&self.cli(&["doctor", "--debug"]).stdout).into_owned()
    }

    /// The `harness_states` block of the named harness (null when the daemon
    /// does not answer).
    fn states(&self, harness: &str) -> Value {
        let doc = self.doctor_json();
        doc["doctor"]["harness_states"]["harnesses"]
            .as_array()
            .and_then(|all| all.iter().find(|h| h["harness"] == harness))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn version_state(&self, harness: &str, version: &str) -> Option<Value> {
        self.states(harness)["versions"]
            .as_array()?
            .iter()
            .find(|v| v["version"] == version)
            .cloned()
    }

    /// Health's limitations and notes, one string each.
    fn health_lines(&self) -> Vec<String> {
        let output = self.cli(&["daemon", "health", "--json"]);
        let doc: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("daemon health --json: {e}"));
        let data = &doc["result"]["data"];
        ["limitations", "notes"]
            .iter()
            .flat_map(|key| data[key].as_array().cloned().unwrap_or_default())
            .filter_map(|line| line.as_str().map(str::to_owned))
            .collect()
    }

    /// Health lines that mention `version` (every harness-version line names it).
    fn version_lines(&self, version: &str) -> Vec<String> {
        self.health_lines()
            .into_iter()
            .filter(|line| line.contains(version))
            .collect()
    }

    fn instance_dir(&self) -> PathBuf {
        PathBuf::from(
            self.doctor_json()["doctor"]["instance_dir"]
                .as_str()
                .expect("doctor reports the instance dir"),
        )
    }

    fn cache_file(&self) -> PathBuf {
        self.instance_dir()
            .join("harness-manifest/harness-versions.json")
    }

    fn cache_meta(&self) -> Option<Value> {
        let text =
            fs::read_to_string(self.instance_dir().join("harness-manifest/meta.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Wait until the daemon's detached fetch has written the cache.
    fn wait_for_cache(&self) {
        wait_for(
            "the manifest cache to be written",
            Duration::from_secs(15),
            || self.cache_file().exists().then_some(()),
        );
    }

    /// Wait until the daemon has logged the manifest policy it runs under
    /// (`expected`, e.g. `off (settings.json)`), then assert nothing was
    /// fetched: no cache file and no recorded attempt. The policy line is
    /// written after election, before any fetch could start; `Off` never
    /// fetches (`opt_out_never_fetches` in tests/harness/manifest.rs). No
    /// sleep: the daemon log is the structural proof.
    fn assert_policy_off(&self, expected: &str) {
        let log = self.instance_dir().join("daemon.log");
        let line = format!("harness manifest: {expected}");
        wait_for(
            "the daemon to log its manifest policy",
            Duration::from_secs(15),
            || {
                fs::read_to_string(&log)
                    .ok()
                    .filter(|text| text.contains(&line))
                    .map(|_| ())
            },
        );
        assert!(!self.cache_file().exists(), "a manifest was fetched");
        let attempts = self
            .cache_meta()
            .map(|meta| meta["attempts"].clone())
            .filter(|attempts| attempts.as_object().is_some_and(|map| !map.is_empty()));
        assert_eq!(attempts, None, "a fetch attempt was recorded");
    }

    fn write_json(&self, name: &str, value: &Value) -> PathBuf {
        let path = self.root.join("work").join(name);
        fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
        path
    }

    // ----- manifests, through the canary writer

    fn manifest_py(&self, args: &[&str]) -> String {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/canary/manifest.py");
        let output = Command::new("python3")
            .arg(script)
            .args(args)
            .arg("--binary")
            .arg(BIN)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "manifest.py {args:?}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// A manifest the canary writer produces from a baseline holding one
    /// verified claude row (`last_working`) and a run where each of `broken`
    /// versions violated `SessionStart.session_id` under this binary's contract.
    fn canary_manifest(&self, name: &str, last_working: &str, broken: &[&str]) -> PathBuf {
        let baseline = self.write_json(
            &format!("{name}-baseline.json"),
            &json!({"schema_version": 2, "generated_from": "baseline", "generated_at": null,
                    "latest_release": null, "contracts": {}, "rows": [
                {"harness": "claude", "version": last_working, "status": "verified",
                 "evidence": "live", "contract_id": null, "source": "manual"}]}),
        );
        let probes: Vec<Value> = broken
            .iter()
            .map(|version| {
                json!({"version": version, "role": "newest", "result": "fail", "flaky": false,
                       "attempts": [{"result": "fail", "contract": {"payloads": [
                           {"event": "SessionStart", "kind": "violation", "field": "session_id"}]}}]})
            })
            .collect();
        let report = self.write_json(
            &format!("{name}-report.json"),
            &json!({"schema_version": 1, "canary_commit": "1".repeat(40), "harnesses": [
                {"harness": "claude", "status": "break", "first_bad": broken[0], "probes": probes}]}),
        );
        let out = self.root.join("work").join(format!("{name}.json"));
        self.manifest_py(&[
            "write",
            "--baseline",
            baseline.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
            "--generated-at",
            "2026-10-02T06:00:00Z",
            "--out",
            out.to_str().unwrap(),
        ]);
        let written: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
        let rows = written["rows"].as_array().unwrap();
        for version in broken {
            assert!(
                rows.iter().any(|r| r["version"] == *version
                    && r["status"] == "known_broken"
                    && r["source"] == "canary"),
                "the writer produced no canary known_broken row for {version}: {written}"
            );
        }
        out
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.cli(&["daemon", "stop"]);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn session_start_then_tool(rig: &Rig, version: &str, session: &str) {
    let transcript = rig.transcript(session, version);
    for event in ["SessionStart", "PreToolUse"] {
        let payload = Rig::payload(event, &transcript, session, &[]);
        let run = rig.hook(Some(event), &payload, false, true);
        assert_eq!(run.code, Some(0), "{event} hook: {}", run.stdout);
    }
}

/// Kills: an unlisted newer version that adds a Health line (before or after
/// its first payloads), or evidence that never reaches `working`.
#[test]
fn unlisted_new_version_is_silent_then_working() {
    let mut rig = Rig::new(Some(NEW_VERSION));
    rig.start(&[]);
    assert_eq!(rig.version_lines(NEW_VERSION), Vec::<String>::new());

    session_start_then_tool(&rig, NEW_VERSION, "s-new");

    let state = wait_for("the version to be working", Duration::from_secs(10), || {
        rig.version_state("claude", NEW_VERSION)
            .filter(|v| v["state"] == "working")
    });
    assert_eq!(
        state["source"],
        "local evidence (lifecycle + tool payloads)"
    );
    assert_eq!(rig.version_lines(NEW_VERSION), Vec::<String>::new());
}

/// Kills: a payload missing a required field that is not degraded, or whose
/// line hides the actual failed input or turns release advice into repair.
#[test]
fn missing_required_field_degrades_without_release_repair_action() {
    if !tools_or_skip("missing_required_field_degrades_without_release_repair_action") {
        return;
    }
    let mut rig = Rig::new(Some(NEW_VERSION));
    let manifest = rig.root.join("work/upgrade.json");
    rig.manifest_py(&[
        "set",
        "--file",
        manifest.to_str().unwrap(),
        "--harness",
        "claude",
        "--version",
        NEW_VERSION,
        "--status",
        "verified",
        "--contract-id",
        OTHER_CONTRACT,
        "--supported-since",
        "99.0.0",
    ]);
    rig.start(&[("HT_TEST_MANIFEST_URL", &Rig::manifest_url(&manifest))]);

    // The first payload creates the evidence row, which makes the daemon fetch
    // the manifest; wait for it before the violation arrives.
    let transcript = rig.transcript("s-miss", NEW_VERSION);
    let start = Rig::payload("SessionStart", &transcript, "s-miss", &[]);
    assert_eq!(
        rig.hook(Some("SessionStart"), &start, false, true).code,
        Some(0)
    );
    rig.wait_for_cache();
    let tool = Rig::payload("PreToolUse", &transcript, "s-miss", &["tool_use_id"]);
    assert_eq!(
        rig.hook(Some("PreToolUse"), &tool, false, true).code,
        Some(0)
    );

    let lines = wait_for("the broken Health line", Duration::from_secs(10), || {
        let lines = rig.version_lines(NEW_VERSION);
        (!lines.is_empty()).then_some(lines)
    });
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0]
            .starts_with("harness claude 2.1.999 contract input failure: PreToolUse/tool_use_id"),
        "{}",
        lines[0]
    );
    assert!(
        !lines[0].contains("upgrade") && !lines[0].contains("pin"),
        "{}",
        lines[0]
    );
    let state = rig.version_state("claude", NEW_VERSION).unwrap();
    assert_eq!(state["state"], "broken");
}

// A captured canary verdict remains diagnostic evidence and cannot veto core Health.
#[test]
fn manifest_known_broken_for_unseen_version_is_advisory() {
    if !tools_or_skip("manifest_known_broken_for_unseen_version_is_advisory") {
        return;
    }
    let mut rig = Rig::new(Some(NEW_VERSION));
    let manifest = rig.canary_manifest("branch", "2.1.900", &[NEW_VERSION]);
    rig.start(&[("HT_TEST_MANIFEST_URL", &Rig::manifest_url(&manifest))]);
    let transcript = rig.transcript("s-mk", NEW_VERSION);
    let start = Rig::payload("SessionStart", &transcript, "s-mk", &[]);
    assert_eq!(
        rig.hook(Some("SessionStart"), &start, false, true).code,
        Some(0)
    );
    let state = wait_for(
        "the historical manifest verdict",
        Duration::from_secs(15),
        || {
            rig.version_state("claude", NEW_VERSION)
                .filter(|state| state["state"] == "broken")
        },
    );
    assert_eq!(state["source"], "canary manifest row");
    assert!(rig.version_lines(NEW_VERSION).is_empty());
    assert!(!state["line"].as_str().unwrap().contains("pin"));
}

/// Kills: a manifest `known_broken` row overriding what has worked on this
/// machine: once both payload kinds arrive the Health line must go, leaving
/// only a doctor note.
#[test]
fn locally_verified_version_stays_working_despite_manifest_known_broken() {
    if !tools_or_skip("locally_verified_version_stays_working_despite_manifest_known_broken") {
        return;
    }
    let mut rig = Rig::new(Some(NEW_VERSION));
    let manifest = rig.canary_manifest("branch", "2.1.900", &[NEW_VERSION]);
    rig.start(&[("HT_TEST_MANIFEST_URL", &Rig::manifest_url(&manifest))]);

    let transcript = rig.transcript("s-lv", NEW_VERSION);
    let start = Rig::payload("SessionStart", &transcript, "s-lv", &[]);
    assert_eq!(
        rig.hook(Some("SessionStart"), &start, false, true).code,
        Some(0)
    );
    // Control: the recorded manifest verdict is intact but only advisory.
    wait_for(
        "the historical broken verdict (control)",
        Duration::from_secs(15),
        || {
            rig.version_state("claude", NEW_VERSION)
                .filter(|state| state["state"] == "broken")
        },
    );
    assert!(rig.version_lines(NEW_VERSION).is_empty());

    let tool = Rig::payload("PreToolUse", &transcript, "s-lv", &[]);
    assert_eq!(
        rig.hook(Some("PreToolUse"), &tool, false, true).code,
        Some(0)
    );
    let state = wait_for("the version to be working", Duration::from_secs(10), || {
        rig.version_state("claude", NEW_VERSION)
            .filter(|v| v["state"] == "working")
    });
    assert_eq!(
        state["source"],
        "local evidence (lifecycle + tool payloads)"
    );
    let notes = state["notes"].to_string();
    assert!(
        notes.contains("canary reports a break in SessionStart/session_id"),
        "{notes}"
    );
    assert_eq!(rig.version_lines(NEW_VERSION), Vec::<String>::new());
    assert!(
        rig.doctor_text().contains("canary reports a break"),
        "doctor shows the note"
    );
}

/// Kills: a daemon that fetches although the instance opted out or the
/// environment says offline. The same manifest drives state when fetching is
/// allowed (previous scenarios), so here the embedded copy decides: the
/// version stays quiet.
#[test]
fn opt_out_and_offline_paths_fetch_nothing() {
    if !tools_or_skip("opt_out_and_offline_paths_fetch_nothing") {
        return;
    }
    // settings.json: {"harness_manifest": "off"}
    let mut rig = Rig::new(Some(NEW_VERSION));
    let manifest = rig.canary_manifest("branch", "2.1.900", &[NEW_VERSION]);
    let url = Rig::manifest_url(&manifest);
    let instance = rig.instance_dir();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&instance)
        .unwrap();
    let settings = instance.join("settings.json");
    fs::write(&settings, r#"{"harness_manifest":"off"}"#).unwrap();
    fs::set_permissions(
        &settings,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    rig.start(&[("HT_TEST_MANIFEST_URL", &url)]);
    let transcript = rig.transcript("s-off", NEW_VERSION);
    let start = Rig::payload("SessionStart", &transcript, "s-off", &[]);
    assert_eq!(
        rig.hook(Some("SessionStart"), &start, false, true).code,
        Some(0)
    );
    wait_for("the evidence row", Duration::from_secs(10), || {
        rig.version_state("claude", NEW_VERSION)
    });
    rig.assert_policy_off("off (settings.json)");
    assert_eq!(rig.version_lines(NEW_VERSION), Vec::<String>::new());
    let policy = &rig.doctor_json()["doctor"]["harness_manifest"];
    assert_eq!(
        (policy["policy"].as_str(), policy["source"].as_str()),
        (Some("off"), Some("settings"))
    );
    drop(rig);

    // HERDR_THREADS_OFFLINE=1 in the daemon's environment
    let mut rig = Rig::new(Some(NEW_VERSION));
    rig.start(&[
        ("HT_TEST_MANIFEST_URL", &url),
        ("HERDR_THREADS_OFFLINE", "1"),
    ]);
    let transcript = rig.transcript("s-offline", NEW_VERSION);
    let start = Rig::payload("SessionStart", &transcript, "s-offline", &[]);
    assert_eq!(
        rig.hook(Some("SessionStart"), &start, false, true).code,
        Some(0)
    );
    wait_for("the evidence row", Duration::from_secs(10), || {
        rig.version_state("claude", NEW_VERSION)
    });
    rig.assert_policy_off("off (HERDR_THREADS_OFFLINE=1)");
    assert_eq!(rig.version_lines(NEW_VERSION), Vec::<String>::new());
}

/// Kills: a truncated or unparseable payload that records an evidence row or
/// adds a Health line.
#[test]
fn truncated_or_unparseable_payload_changes_nothing() {
    let mut rig = Rig::new(Some(NEW_VERSION));
    rig.start(&[]);
    let transcript = rig.transcript("s-bad", NEW_VERSION);
    let whole = Rig::payload("SessionStart", &transcript, "s-bad", &[]);
    let truncated = &whole[..whole.len() / 2];
    for stdin in [truncated, b"{not json".as_slice()] {
        let run = rig.hook(Some("SessionStart"), stdin, false, true);
        assert_eq!(run.code, Some(0), "the hook fails open: {}", run.stdout);
    }
    assert_eq!(rig.states("claude")["versions"], json!([]));
    assert_eq!(rig.version_lines(NEW_VERSION), Vec::<String>::new());
    assert!(
        rig.health_lines()
            .iter()
            .all(|l| !l.contains("hook payload")),
        "{:?}",
        rig.health_lines()
    );
}

/// Kills: a payload whose transcript cannot be read that records a row, or a
/// doctor that does not say why no version evidence exists.
#[test]
fn unattributable_payload_records_nothing_and_doctor_says_why() {
    let mut rig = Rig::new(Some(NEW_VERSION));
    rig.start(&[]);
    let missing = rig.root.join("work/no-such-transcript.jsonl");
    let payload = Rig::payload("SessionStart", &missing, "s-gone", &[]);
    assert_eq!(
        rig.hook(Some("SessionStart"), &payload, false, true).code,
        Some(0)
    );

    let text = wait_for("doctor to name the reason", Duration::from_secs(10), || {
        let text = rig.doctor_text();
        text.contains("version evidence unavailable: transcript not found")
            .then_some(text)
    });
    assert!(
        text.contains("version evidence unavailable: transcript not found ("),
        "{text}"
    );
    assert_eq!(rig.states("claude")["versions"], json!([]));
    assert_eq!(rig.version_lines(NEW_VERSION), Vec::<String>::new());
}

/// Kills: a version that only ever delivers malformed payloads escaping a
/// canary `known_broken` row (it has no way to verify, so the row must win).
#[test]
fn malformed_only_version_with_canary_known_broken_is_broken() {
    if !tools_or_skip("malformed_only_version_with_canary_known_broken_is_broken") {
        return;
    }
    let mut rig = Rig::new(Some(OTHER_VERSION));
    let manifest = rig.canary_manifest("branch", "2.1.900", &[OTHER_VERSION]);
    rig.start(&[("HT_TEST_MANIFEST_URL", &Rig::manifest_url(&manifest))]);

    // An unknown event under no registration: attributed through the valid
    // transcript, classified Malformed.
    let transcript = rig.transcript("s-mal", OTHER_VERSION);
    let mut payload: Value =
        serde_json::from_slice(&Rig::payload("SessionStart", &transcript, "s-mal", &[])).unwrap();
    payload["hook_event_name"] = json!("Bogus");
    let run = rig.hook(None, &serde_json::to_vec(&payload).unwrap(), false, true);
    assert_eq!(run.code, Some(0), "{}", run.stdout);

    let state = wait_for(
        "the historical broken verdict",
        Duration::from_secs(15),
        || {
            rig.version_state("claude", OTHER_VERSION)
                .filter(|state| state["state"] == "broken")
        },
    );
    assert_eq!(state["source"], "canary manifest row");
    assert!(rig.version_lines(OTHER_VERSION).is_empty());
    let db = rusqlite::Connection::open(
        rig.instance_dir()
            .join(herdr_threads::daemon::paths::DATABASE_FILE),
    )
    .unwrap();
    let milestones: (Option<i64>, Option<i64>) = db
        .query_row(
            "SELECT lifecycle_ok_at,tool_ok_at FROM harness_version_evidence WHERE version=?1",
            [OTHER_VERSION],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(milestones, (None, None));
}

/// Kills: an unreachable manifest URL that loses the evidence, skips the
/// fetch attempt, leaves a cache behind or shows up in Health. The hook's
/// independence from the fetch is proven structurally by
/// `hook_never_waits_for_the_manifest_fetch`, not by wall time here.
#[test]
fn unreachable_manifest_url_falls_back_quietly_within_budget() {
    if !tools_or_skip("unreachable_manifest_url_falls_back_quietly_within_budget") {
        return;
    }
    let mut rig = Rig::new(Some(NEW_VERSION));
    rig.start(&[("HT_TEST_MANIFEST_URL", "http://127.0.0.1:9/")]);
    let transcript = rig.transcript("s-unreach", NEW_VERSION);
    for event in ["SessionStart", "PreToolUse"] {
        let payload = Rig::payload(event, &transcript, "s-unreach", &[]);
        let run = rig.hook(Some(event), &payload, false, true);
        assert_eq!(run.code, Some(0));
    }
    let state = wait_for("the version to be working", Duration::from_secs(10), || {
        rig.version_state("claude", NEW_VERSION)
            .filter(|v| v["state"] == "working")
    });
    assert_eq!(
        state["source"],
        "local evidence (lifecycle + tool payloads)"
    );
    // The fetch was attempted (and failed quietly), not skipped.
    wait_for("the failed fetch attempt", Duration::from_secs(10), || {
        rig.cache_meta()
            .filter(|meta| meta["attempts"]["claude"].is_number())
    });
    assert!(!rig.cache_file().exists());
    assert_eq!(rig.version_lines(NEW_VERSION), Vec::<String>::new());
}

/// Kills: a test curl wrapper that leaves the short timeout active, loses
/// fetch arguments or file paths, or fails to execute the real HTTP fetch.
#[test]
fn curl_wrapper_preserves_fetch_arguments_and_stretches_timeout() {
    if !tools_or_skip("curl_wrapper_preserves_fetch_arguments_and_stretches_timeout") {
        return;
    }
    let rig = Rig::new(None);
    let wrapper = rig.curl_with_test_timeout();
    let expected = b"a real curl response";
    let mut server = GatedServer::start(expected.to_vec());
    let body = rig.root.join("work/body with spaces");
    let headers = rig.root.join("work/headers with spaces");
    let mut command = herdr_threads::test_support::spawn::command(wrapper);
    let mut curl = command
        .env("HOME", rig.root.join("home"))
        .args([
            "-fsS",
            "--max-time",
            "0.001",
            "--max-filesize",
            "1024",
            "-o",
        ])
        .arg(&body)
        .arg("-D")
        .arg(&headers)
        .args(["-w", "%{http_code}", &server.url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_owned()
        .unwrap();
    wait_for("curl's HTTP request", Duration::from_secs(15), || {
        assert!(
            curl.try_wait().unwrap().is_none(),
            "curl exited before the held response was released"
        );
        (server.accepted.load(Ordering::SeqCst) > 0).then_some(())
    });
    server.release_and_join();
    let output = curl.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"200");
    assert_eq!(fs::read(body).unwrap(), expected);
    assert!(
        fs::read_to_string(headers)
            .unwrap()
            .contains("ETag: \"gated\"")
    );
}

/// Kills: a hook (or the daemon's reply to it) that waits for the manifest
/// fetch. The fetch is held open by a gated server; the daemon has answered
/// the hook, and the hook has written its gate, while the fetch is still
/// held.
#[test]
fn hook_never_waits_for_the_manifest_fetch() {
    if !tools_or_skip("hook_never_waits_for_the_manifest_fetch") {
        return;
    }
    let mut rig = Rig::new(Some(NEW_VERSION));
    rig.curl_with_test_timeout();
    let manifest = rig.canary_manifest("gated", "2.1.900", &[NEW_VERSION]);
    let mut server = GatedServer::start(fs::read(&manifest).unwrap());
    rig.start(&[("HT_TEST_MANIFEST_URL", &server.url)]);
    let transcript = rig.transcript("s-gated", NEW_VERSION);
    let start = Rig::payload("SessionStart", &transcript, "s-gated", &[]);
    let gate = herdr_threads::cli::hook_evidence::gate_path(
        &herdr_threads::cli::hook_evidence::gate_dir(&rig.state),
        "claude",
        "s-gated",
    );
    // Resolve doctor-derived paths before the hook starts the held fetch.
    let cache = rig.cache_file();
    let run = rig.hook(Some("SessionStart"), &start, false, true);
    assert_eq!(run.code, Some(0), "{}", run.stdout);
    wait_for(
        "the daemon's fetch to reach the server",
        Duration::from_secs(15),
        || (server.accepted.load(Ordering::SeqCst) > 0).then_some(()),
    );
    // The hook has exited and its gate is written only after the daemon's
    // reply. The server still holds the response, so neither waited for it.
    let stored: Value = serde_json::from_slice(&fs::read(&gate).unwrap()).unwrap();
    assert!(
        stored["heartbeat_at_ms"].is_number(),
        "the gate holds no heartbeat: {stored}"
    );
    assert!(
        !cache.exists(),
        "the manifest landed before the fetch was released"
    );
    server.release_and_join();
    wait_for(
        "the manifest cache to be written",
        Duration::from_secs(15),
        || cache.exists().then_some(()),
    );
}

// Runtime-written metadata remains optional and cannot become a PATH witness.
#[test]
fn attributed_version_does_not_invent_daemon_detected_metadata() {
    let mut rig = Rig::new(Some(NEW_VERSION));
    rig.start(&[]);
    assert!(rig.states("claude")["detected"].is_null());
    session_start_then_tool(&rig, NEW_VERSION, "s-det");
    let row = wait_for("the attributed row", Duration::from_secs(10), || {
        rig.states("claude")["versions"][0]
            .is_object()
            .then(|| rig.states("claude")["versions"][0].clone())
    });
    assert_eq!(row["version"], NEW_VERSION);
    assert!(
        row["notes"]
            .to_string()
            .contains("runtime-written Claude transcript")
    );
    assert!(rig.states("claude")["detected"].is_null());
}

// Historical floor classification is retained without refusing core callbacks.
#[test]
fn below_floor_version_evidence_is_advisory() {
    const OLD: &str = "1.0.0";
    assert!(matches!(
        herdr_threads::harness::state::ladder_for("claude", OLD),
        Ladder::BelowFloor { .. }
    ));
    let mut rig = Rig::new(Some(OLD));
    rig.start(&[]);
    let transcript = rig.transcript("s-old", OLD);
    let start = Rig::payload("SessionStart", &transcript, "s-old", &[]);
    let run = rig.hook(Some("SessionStart"), &start, true, true);
    assert_eq!(run.code, Some(0), "{}", run.stdout);
    let state = wait_for(
        "the historical below-floor verdict",
        Duration::from_secs(10),
        || rig.version_state("claude", OLD),
    );
    assert_eq!(state["state"], "broken");
    assert_eq!(state["source"], "below the recipe floor");
    assert!(rig.version_lines(OLD).is_empty());
}
