//! ht-4ne.3: configuration smoke for bare Hermes bridge enrollment (spec D9).
//!
//! The bridge is installed with `setup hermes --profile work` into a temp
//! `HERMES_HOME`, then the INSTALLED module (`tests/fixtures/hermes/smoke_driver.py`)
//! is driven with synthetic Hermes 0.21.5-shaped callbacks. Its real hook child
//! (`herdr-threads hook hermes`) talks to an isolated named Herdr session and an
//! isolated daemon/state dir; assertions read the daemon's database. No model, no
//! installed Hermes, never `~/.hermes` or the shared Herdr server.
use herdr_threads::test_support::{
    isolated_herdr::IsolatedHerdr,
    isolation::scrub_env,
    spawn::{self, SpawnOwned},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const DRIVER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/hermes/smoke_driver.py"
);

/// The pane-bound observation row we assert on: a Hermes top-level binding made
/// by the cooperative path (never by launch or by a person).
const OPEN_HERMES: &str = "b.harness='hermes' AND b.observation_provenance='cooperative_top_level' AND b.ended_at IS NULL";

struct Smoke {
    herdr: IsolatedHerdr,
    state: PathBuf,
    native_home: PathBuf,
    native_src: PathBuf,
    launcher: PathBuf,
    /// The installed bridge: `<native home>/profiles/work/plugins/herdr-threads/__init__.py`.
    bridge: PathBuf,
    cwd: PathBuf,
    next_label: usize,
}

struct Driven {
    /// The installed module's own `CHILD_RESTRICTION`.
    child_restriction: String,
    steps: Vec<Value>,
}

impl Driven {
    fn result(&self, step: usize) -> &Value {
        &self.steps[step]["result"]
    }
    /// The `lifecycle_ack.mode` the bridge validated for this step (the last
    /// hook answer it received), `None` when the step made no hook call.
    fn ack_mode(&self, step: usize) -> Option<String> {
        self.steps[step]["child"]
            .as_array()?
            .last()?
            .get("lifecycle_ack")?
            .get("mode")?
            .as_str()
            .map(str::to_owned)
    }
    fn context(&self, step: usize) -> Option<&str> {
        self.result(step)["context"]
            .as_str()
            .filter(|c| !c.is_empty())
    }
}

impl Smoke {
    /// An isolated Herdr, state dir, synthetic Hermes tree and the installed
    /// bridge. `ensure_daemon` starts the isolated daemon before any hook.
    /// `None` (after `IsolatedHerdr` prints `skipped: no herdr: <case>`) when
    /// herdr is absent and `HT_SKIP_HERDR_TESTS=1`; panics naming `herdr`
    /// otherwise. Pass the test's own name as `case`.
    fn new(case: &'static str, ensure_daemon: bool) -> Option<Self> {
        let herdr = IsolatedHerdr::new(case)?;
        let root = herdr.root().to_path_buf();
        fs::create_dir_all(root.join("cfg")).unwrap();
        fs::write(root.join("cfg/herdr.toml"), "default_shell = '/bin/sh'\n").unwrap();
        let state = root.join("plugin-state");
        let smoke = {
            let (native_home, native_src, launcher) = synthetic_hermes(&root);
            let bridge = native_home.join("profiles/work/plugins/herdr-threads/__init__.py");
            Smoke {
                herdr,
                state,
                native_home,
                native_src,
                launcher,
                bridge,
                cwd: root.join("home"),
                next_label: 0,
            }
        };
        fs::create_dir_all(&smoke.cwd).unwrap();
        smoke.write_settings();
        smoke.herdr.start();
        // `setup` takes every option after the subcommand.
        let mut setup = smoke.base(spawn::command(BIN));
        spawn::tag(&mut setup);
        let setup = setup
            .args(["setup", "hermes", "--json", "--state-dir"])
            .arg(&smoke.state)
            .arg("--host-endpoint")
            .arg(smoke.herdr.socket_path())
            .args(["--profile", "work", "--harness-binary"])
            .arg(&smoke.launcher)
            .output()
            .unwrap();
        assert!(
            setup.status.success(),
            "setup hermes: {}{}",
            String::from_utf8_lossy(&setup.stdout),
            String::from_utf8_lossy(&setup.stderr)
        );
        if ensure_daemon {
            smoke.ok(&["daemon", "ensure"]);
        }
        Some(smoke)
    }

    /// `{"harness_manifest":"off"}` before anything starts the daemon, so the
    /// bare Hermes callback admission does not depend on the embedded manifest.
    fn write_settings(&self) {
        let paths = self.paths();
        paths.prepare_instance_dir().unwrap();
        let settings = paths
            .instance_dir
            .join(herdr_threads::daemon::settings::SETTINGS_FILE);
        fs::write(&settings, br#"{"harness_manifest":"off"}"#).unwrap();
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn paths(&self) -> herdr_threads::daemon::paths::InstancePaths {
        let runtime = herdr_threads::daemon::paths::RuntimeContext::explicit(
            self.state.clone(),
            self.herdr.socket_path(),
            None,
        )
        .unwrap();
        herdr_threads::daemon::paths::InstancePaths::resolve(&runtime).unwrap()
    }

    /// The private environment every child gets: scrubbed, isolated HOME and
    /// config homes under the Herdr root, never the real ones.
    fn base(&self, mut command: Command) -> Command {
        let root = self.herdr.root();
        scrub_env(&mut command);
        command
            .envs(self.herdr.env())
            .env("CLAUDE_CONFIG_DIR", root.join("claude"))
            .env("CODEX_HOME", root.join("codex"))
            .env("HERMES_HOME", &self.native_home)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_ENV");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HERDR_") && key != "HERDR_THREADS_TEST_OWNER_PID"
            {
                command.env_remove(key);
            }
        }
        command.envs(self.herdr.env());
        command
    }

    fn cli(&self) -> Command {
        let mut command = self.base(spawn::command(BIN));
        spawn::tag(&mut command);
        command
            .arg("--json")
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(self.herdr.socket_path());
        command
    }

    fn ok(&self, args: &[&str]) -> Value {
        let out = self.cli().args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
    }

    fn daemon_up(&self) -> bool {
        self.cli()
            .args(["daemon", "health"])
            .output()
            .unwrap()
            .status
            .success()
    }

    /// A fresh `herdr workspace create` root pane (never a pane we did not make).
    fn pane(&mut self) -> String {
        self.next_label += 1;
        let label = format!("hermes-smoke-{}", self.next_label);
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

    /// One driver process (one Hermes process) replaying `steps`.
    fn drive(&self, pane: &str, steps: &[Value], in_herdr: bool) -> Driven {
        let mut command = self.base(Command::new("python3"));
        // The driver is the Hermes process: the hooks it runs have it as their harness parent.
        command
            .env("HT_TEST_HARNESS", "hermes")
            .args(["-I", DRIVER])
            .arg(&self.bridge)
            .arg(&self.native_src)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if in_herdr {
            command
                .env("HERDR_ENV", "1")
                .env("HERDR_PANE_ID", pane)
                .env("HERDR_SOCKET_PATH", self.herdr.socket_path());
        } else {
            command.env_remove("HERDR_SOCKET_PATH");
        }
        let mut child = command.spawn_owned().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(json!({ "steps": steps }).to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "driver failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let lines = String::from_utf8(out.stdout).unwrap();
        let mut parsed: Vec<Value> = lines
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
            .collect();
        let head = parsed.remove(0);
        assert!(
            out.stderr.is_empty(),
            "the bridge surfaced stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(parsed.len(), steps.len(), "driver answered every step");
        Driven {
            child_restriction: head["bridge"]["child_restriction"]
                .as_str()
                .unwrap()
                .to_owned(),
            steps: parsed,
        }
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.paths().database_path).unwrap()
    }

    fn count(&self, sql: &str, params: &[&dyn rusqlite::ToSql]) -> i64 {
        self.db().query_row(sql, params, |row| row.get(0)).unwrap()
    }

    fn total_seats(&self) -> i64 {
        self.count("SELECT COUNT(*) FROM seats", &[])
    }

    fn seat_ids(&self, pane: &str) -> Vec<String> {
        let db = self.db();
        let mut statement = db
            .prepare("SELECT id FROM seats WHERE target_id = ?1 ORDER BY ordinal")
            .unwrap();
        statement
            .query_map([pane], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn seat(&self, pane: &str) -> String {
        let seats = self.seat_ids(pane);
        assert_eq!(seats.len(), 1, "one seat for {pane}: {seats:?}");
        seats[0].clone()
    }

    fn open_bindings(&self, pane: &str) -> i64 {
        self.count(
            &format!(
                "SELECT COUNT(*) FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id \
                 WHERE s.target_id=?1 AND {OPEN_HERMES}"
            ),
            &[&pane],
        )
    }

    /// The pane's open Hermes bindings in ordinal order:
    /// (native session, liveness, disconnect reason, role).
    fn hermes_rows(&self, pane: &str) -> Vec<(String, String, Option<String>, String)> {
        let db = self.db();
        let mut statement = db
            .prepare(
                "SELECT b.native_session, b.liveness, b.disconnect_reason, b.role \
                 FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id \
                 WHERE s.target_id=?1 AND b.harness='hermes' AND b.ended_at IS NULL \
                 ORDER BY b.ordinal",
            )
            .unwrap();
        statement
            .query_map([pane], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn all_bindings(&self, pane: &str) -> i64 {
        self.count(
            "SELECT COUNT(*) FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id \
             WHERE s.target_id=?1 AND b.harness='hermes'",
            &[&pane],
        )
    }

    /// The native session of the pane's one open Hermes binding.
    fn open_session(&self, pane: &str) -> String {
        self.db()
            .query_row(
                &format!(
                    "SELECT b.native_session FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id \
                     WHERE s.target_id=?1 AND {OPEN_HERMES} ORDER BY b.ordinal DESC LIMIT 1"
                ),
                [pane],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn wait_daemon_up(&self, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.daemon_up() {
            assert!(Instant::now() < deadline, "{what}: daemon never came up");
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// `herdr-threads hook hermes` fed one envelope directly (configuration 7).
    fn hook(&self, pane: &str, envelope: &Value) -> (i32, Vec<u8>, Vec<u8>) {
        let mut command = self.base(spawn::command(BIN));
        command
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(self.herdr.socket_path())
            .args(["hook", "hermes"])
            .env("HT_TEST_HARNESS", "hermes")
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", pane)
            .env("HERDR_SOCKET_PATH", self.herdr.socket_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn_owned().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(envelope).unwrap())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        (out.status.code().unwrap_or(-1), out.stdout, out.stderr)
    }
}

impl Drop for Smoke {
    fn drop(&mut self) {
        // Every daemon this smoke (or a hook it ran) started stops before the
        // test ends; the IsolatedHerdr teardown that follows stops the server.
        let _ = self.cli().args(["daemon", "stop"]).output();
    }
}

/// The synthetic Hermes source tree and launcher that `setup hermes --profile work
/// --harness-binary` accepts (the shape of `tests/setup_cli.rs::HermesScopeFixture`,
/// which is not shared code). Returns (native home, native source root, launcher).
fn synthetic_hermes(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let mut python = herdr_threads::test_support::spawn::command("python3");
    python
        .args(["-c", "import sys;print(sys.executable)"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = python.spawn_owned().unwrap().wait_with_output().unwrap();
    assert!(output.status.success());
    let interpreter = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        .canonicalize()
        .unwrap();
    let source = root.join("synthetic source");
    let package = source.join("hermes_cli");
    let dependencies = root.join("dependencies/site-packages");
    let native_home = root.join("native home with spaces");
    fs::create_dir_all(&package).unwrap();
    fs::create_dir_all(&dependencies).unwrap();
    fs::create_dir_all(native_home.join("profiles/work")).unwrap();
    let py = |p: &Path| serde_json::to_string(p.to_str().unwrap()).unwrap();
    for (path, bytes) in [
        (source.join("hermes_bootstrap.py"), format!("import os,sys\nfrom pathlib import Path\n_root=Path(__file__).resolve().parent\n_pm_repair=False\n_launch_python=None\nsys.path[:]=[str(_root),{},*sys.path[1:]]\nos.environ['PYTHONPATH']=os.pathsep.join(sys.path[:2])\n", py(&dependencies))),
        (source.join("hermes_constants.py"), "import os\nfrom pathlib import Path\ndef get_hermes_home(): return Path(os.environ['HERMES_HOME'])\n".into()),
        (package.join("__init__.py"), String::new()),
        (package.join("profiles.py"), format!("from pathlib import Path\ndef normalize_profile_name(p): return p.strip().lower()\ndef validate_profile_name(p):\n if p not in ('default','work'): raise ValueError('private')\ndef resolve_profile_env(p):\n p=normalize_profile_name(p);validate_profile_name(p)\n root=Path({});home=root if p=='default' else root/'profiles'/p\n if not home.is_dir(): raise FileNotFoundError('private')\n return str(home)\n", py(&native_home))),
        (package.join("version_info.py"), "from types import SimpleNamespace\ndef get_version_info(): return SimpleNamespace(source='git',base_version='0.21.5',derived_version='0.21.5+1.g1234567',commit='1234567890abcdef1234567890abcdef12345678',dirty=False,distance=1)\n".into()),
        (package.join("config.py"), "class FailedConfigRead(dict): pass\ndef load_config_readonly(): return {'plugins':{'enabled':['herdr-threads'],'disabled':[]}}\n".into()),
    ] {
        fs::write(path, bytes).unwrap();
    }
    for home in [&native_home, native_home.join("profiles/work").as_path()] {
        fs::write(
            home.join("config.yaml"),
            b"plugins:\n  enabled: [herdr-threads]\nother: preserved\n",
        )
        .unwrap();
    }
    let bootstrap = format!(
        "import sys,runpy;sys.path.insert(0,{});import hermes_bootstrap;runpy.run_module('trace',run_name='__main__',alter_sys=True)",
        py(&source)
    );
    let calls = root.join("selected-launcher-calls.jsonl");
    let launcher = root.join("selected launcher");
    fs::write(&launcher, format!("#!{}\nimport json,sys,os\nwith open({},'a') as f: f.write(json.dumps({{'argv':sys.argv[1:],'home':os.environ['HOME'],'native_home':os.environ.get('HERMES_HOME')}})+'\\n')\nprint(json.dumps([{},'-I','-c',{},'--count','--no-report',sys.argv[-3],'--profile',sys.argv[-1]]))\n", interpreter.display(), py(&calls), py(&interpreter), serde_json::to_string(&bootstrap).unwrap())).unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
    (native_home, source, launcher)
}

// ---- synthetic Hermes 0.21.5 callbacks -------------------------------------

fn callback(callback: &str, kwargs: Value) -> Value {
    json!({ "callback": callback, "kwargs": kwargs })
}

/// `on_session_start`: Hermes 0.21.5 `agent/conversation_loop.py:833-835` invokes
/// `invoke_hook("on_session_start", session_id=..., model=..., platform=...)`.
/// A missing platform is coerced to "" by Hermes; `None` here omits the key.
fn session_start(session: &str, platform: Option<&str>) -> Value {
    let mut kwargs = json!({ "session_id": session, "model": "m" });
    if let Some(platform) = platform {
        kwargs["platform"] = platform.into();
    }
    callback("on_session_start", kwargs)
}

/// `pre_llm_call`: the turn-context kwargs of `agent/turn_context.py:761-771`;
/// the key set is the `qualified` case of `integrations/hermes/testdata/callbacks.json`.
/// A top-level turn has `parent_session_id == ""`; `repeat_if_slow` is the retry
/// rule of `config_smoke.rs::hermes_hook_with_context` (an overrun is replayed).
fn turn(session: &str, turn_id: &str) -> Value {
    let all: Value = serde_json::from_str(include_str!(
        "../../integrations/hermes/testdata/callbacks.json"
    ))
    .unwrap();
    let mut kwargs = all["qualified"].clone();
    kwargs["session_id"] = session.into();
    kwargs["turn_id"] = turn_id.into();
    kwargs["task_id"] = format!("task-{turn_id}").into();
    let mut step = callback("pre_llm_call", kwargs);
    step["repeat_if_slow"] = true.into();
    step
}

/// A delegate child: `tools/delegate_tool.py:239` runs children with
/// `platform="subagent"` and the parent's session as `parent_session_id`.
fn child_turn(session: &str, parent: &str) -> Value {
    let all: Value = serde_json::from_str(include_str!(
        "../../integrations/hermes/testdata/callbacks.json"
    ))
    .unwrap();
    let mut kwargs = all["child"].clone();
    kwargs["session_id"] = session.into();
    kwargs["parent_session_id"] = parent.into();
    callback("pre_llm_call", kwargs)
}

/// `on_session_reset`: `hermes_cli/cli_session_mixin.py:428-441` (`/new`). It names
/// the NEW session (the reset hint targets the next turn of that session; see the
/// harness-adapters design, "records bounded pending Clear metadata for the newly
/// named session").
fn session_reset(session: &str) -> Value {
    callback(
        "on_session_reset",
        json!({ "session_id": session, "reason": "new_session", "platform": "cli" }),
    )
}

fn assert_seated_turn(driven: &Driven, step: usize, mode: &str) {
    assert!(
        driven.context(step).is_some(),
        "step {step}: the first turn delivered no context: {}",
        driven.steps[step]
    );
    assert_eq!(
        driven.ack_mode(step).as_deref(),
        Some(mode),
        "step {step}: {}",
        driven.steps[step]
    );
}

#[test]
fn smoke_installs_the_enrollment_bridge() {
    let Some(smoke) = Smoke::new("smoke_installs_the_enrollment_bridge", true) else {
        return;
    };
    let installed = fs::read_to_string(&smoke.bridge).unwrap();
    assert!(
        installed.contains("enrollment_witness"),
        "stale bridge installed"
    );
    assert!(installed.contains("cli_session_start"));
    let config: Value = serde_json::from_slice(
        &fs::read(smoke.bridge.with_file_name("bridge_config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(config["rust_executable"], BIN);
    assert_eq!(
        config["host_endpoint"],
        smoke.herdr.socket_path().to_str().unwrap()
    );
    // Loads and registers under the stub context: all four hooks.
    let pane = "unused";
    let driven = smoke.drive(pane, &[], false);
    assert!(
        driven
            .child_restriction
            .starts_with("Subagents may discover")
    );
}

/// A new pane seated through the bridge: a CLI start then a top-level turn in
/// one Hermes process. Returns the pane and its seat.
fn seated_pane(smoke: &mut Smoke, session: &str) -> (String, String) {
    let pane = smoke.pane();
    let driven = smoke.drive(
        &pane,
        &[session_start(session, Some("cli")), turn(session, "T1")],
        true,
    );
    assert_seated_turn(&driven, 1, "startup");
    (pane.clone(), smoke.seat(&pane))
}

/// A legacy (pre-ht-4ne.1) envelope: no `enrollment_witness` anywhere, as an
/// older installed bridge sends it; fresh timestamps like `config_smoke.rs`.
fn legacy_envelope(callback: &str, session: &str, event: &str) -> Value {
    let mut payload: Value =
        serde_json::from_slice(include_bytes!("../fixtures/hermes/envelopes.json")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    payload["callback"] = callback.into();
    payload["session_id"] = session.into();
    payload["event_id"] = event.into();
    payload["started_at"] = now.into();
    payload["deadline_at"] = (now
        + if callback == "pre_llm_call" {
            1200
        } else {
            4500
        })
    .into();
    payload["observation_order"]["observed_at_millis"] = now.into();
    if callback == "pre_llm_call" {
        payload["turn_id"] = event.into();
        payload["role_association"]["session_id"] = session.into();
        payload["role_association"]["turn_id"] = event.into();
    } else {
        payload["turn_id"] = Value::Null;
        payload["role_association"] = Value::Null;
        payload["observation_order"]["callback_budget_millis"] = 4500.into();
    }
    assert!(
        !payload.to_string().contains("enrollment_witness"),
        "the legacy fixture must not carry the witness"
    );
    payload
}

#[test]
fn hermes_bare_enrollment_configurations_with_daemon_up() {
    let Some(mut smoke) = Smoke::new("hermes_bare_enrollment_configurations_with_daemon_up", true)
    else {
        return;
    };

    // 1a. CLI start alone: the pane gets its seat, nothing checks in.
    let pane = smoke.pane();
    let started = smoke.drive(&pane, &[session_start("S1", Some("cli"))], true);
    assert!(started.result(0).is_null(), "{}", started.steps[0]);
    let seat = smoke.seat(&pane);
    assert_eq!(smoke.open_bindings(&pane), 0, "a start must not check in");
    assert_eq!(smoke.all_bindings(&pane), 0);
    // 1b. In one process: start enrolls, the first top-level turn checks in and
    // delivers context (a seat alone is not enough).
    let (pane_1b, seat_1b) = seated_pane(&mut smoke, "S1b");
    assert_ne!(seat, seat_1b, "each pane has its own seat");
    assert_eq!(smoke.open_bindings(&pane_1b), 1);
    assert_eq!(smoke.open_session(&pane_1b), "S1b");
    // The start-enrolled pane: its first turn (a later process) checks in on the same seat.
    let later = smoke.drive(&pane, &[turn("S1", "T1")], true);
    assert_seated_turn(&later, 0, "startup");
    assert_eq!(smoke.seat(&pane), seat);
    assert_eq!(smoke.open_bindings(&pane), 1);

    // 2. A delegate child: no seat, only the child restriction.
    let pane = smoke.pane();
    let seats_before = smoke.total_seats();
    let child = smoke.drive(
        &pane,
        &[
            session_start("child", Some("subagent")),
            child_turn("child", "parent"),
        ],
        true,
    );
    assert!(child.result(0).is_null());
    assert_eq!(
        child.result(1),
        &json!({ "context": child.child_restriction })
    );
    assert!(smoke.seat_ids(&pane).is_empty());
    assert_eq!(
        smoke.total_seats(),
        seats_before,
        "a subagent allocated a seat"
    );

    // 3. Platform "" and a missing platform (Hermes coerces None to ""): no enrollment.
    for platform in [Some(""), None] {
        let pane = smoke.pane();
        let driven = smoke.drive(&pane, &[session_start("S3", platform)], true);
        assert!(driven.result(0).is_null());
        assert!(
            smoke.seat_ids(&pane).is_empty(),
            "platform {platform:?} enrolled"
        );
    }
    assert_eq!(
        smoke.total_seats(),
        seats_before,
        "no seat for platform \"\"/None"
    );

    // 4. /new: reset, a CLI start for the new session, its first turn: same seat, Clear.
    let (pane, seat) = seated_pane(&mut smoke, "S4a");
    let new = smoke.drive(
        &pane,
        &[
            session_reset("S4b"),
            session_start("S4b", Some("cli")),
            turn("S4b", "T1"),
        ],
        true,
    );
    assert_seated_turn(&new, 2, "clear");
    assert_eq!(smoke.seat(&pane), seat, "/new moved the seat");
    // Each session keeps its own binding (spec §1): /new adds one and ends nothing.
    assert_eq!(smoke.all_bindings(&pane), 2);
    assert_eq!(smoke.open_bindings(&pane), 2);
    assert_eq!(smoke.open_session(&pane), "S4b");

    // 5. Fallback: a top-level turn with no start in an unseated pane enrolls and
    // checks in within that one turn.
    let pane = smoke.pane();
    let fallback = smoke.drive(&pane, &[turn("S5", "T1")], true);
    assert_seated_turn(&fallback, 0, "startup");
    smoke.seat(&pane);
    assert_eq!(smoke.open_bindings(&pane), 1);

    // 6. Outside Herdr: silent, nothing allocated anywhere.
    let pane = smoke.pane();
    let seats_before = smoke.total_seats();
    let outside = smoke.drive(
        &pane,
        &[session_start("S6", Some("cli")), turn("S6", "T1")],
        false,
    );
    assert!(
        outside.result(0).is_null() && outside.result(1).is_null(),
        "{:?}",
        outside.steps
    );
    assert_eq!(smoke.total_seats(), seats_before);
    assert!(smoke.seat_ids(&pane).is_empty());

    // 7. An older bridge's legacy envelopes (no enrollment witness), straight
    // through the Rust hook: no enrollment and nothing refused on an unseated
    // pane; on a launch-created seat (`seat resolve`) the turn still checks in.
    let pane = smoke.pane();
    let seats_before = smoke.total_seats();
    for envelope in [
        legacy_envelope("on_session_start", "S7", "legacy-start"),
        legacy_envelope("pre_llm_call", "S7", "legacy-turn-unseated"),
    ] {
        let (code, stdout, stderr) = smoke.hook(&pane, &envelope);
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&stderr));
        let answer: Value = if stdout.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&stdout).unwrap()
        };
        assert!(
            answer["context"].is_null(),
            "unseated legacy answer: {answer}"
        );
    }
    assert_eq!(
        smoke.total_seats(),
        seats_before,
        "a legacy envelope enrolled"
    );
    assert!(smoke.seat_ids(&pane).is_empty());
    let resolved = smoke.ok(&["seat", "resolve", "--pane", &pane]);
    let launch_seat = resolved["result"]["data"].as_str().unwrap().to_owned();
    let mut checked_in = Value::Null;
    for attempt in 0..4 {
        let started = Instant::now();
        let envelope = legacy_envelope("pre_llm_call", "S7", "legacy-turn-seated");
        let (code, stdout, stderr) = smoke.hook(&pane, &envelope);
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&stderr));
        checked_in = serde_json::from_slice(&stdout).unwrap_or(Value::Null);
        if !checked_in["context"].is_null() || started.elapsed() < Duration::from_secs(1) {
            break;
        }
        assert!(attempt < 3);
    }
    assert!(
        checked_in["context"].is_string(),
        "legacy turn on a launch seat: {checked_in}"
    );
    assert_eq!(smoke.seat(&pane), launch_seat);
    assert_eq!(smoke.open_bindings(&pane), 1);

    // 9. Resume: (a) a turn for an existing session id with no start, unseated
    // pane: fallback enrollment and check-in.
    let pane = smoke.pane();
    let resumed = smoke.drive(&pane, &[turn("resumed-S", "T1")], true);
    assert_seated_turn(&resumed, 0, "startup");
    smoke.seat(&pane);
    // (b) a seated pane, a new Hermes process (`hermes --resume OTHER`): a turn for
    // a pre-existing session with no start checks in as Startup on the same seat
    // (spec D5): a new binding, no new seat.
    let (pane, seat) = seated_pane(&mut smoke, "S9");
    let resumed = smoke.drive(&pane, &[turn("S9-older", "T9")], true);
    assert_seated_turn(&resumed, 0, "startup");
    assert_eq!(smoke.seat_ids(&pane), vec![seat.clone()]);
    assert_eq!(smoke.all_bindings(&pane), 2);
    assert_eq!(
        smoke.open_bindings(&pane),
        2,
        "a different session in a new process is its own binding; nothing ended it (spec decision 3, section 1)"
    );
    assert_eq!(smoke.open_session(&pane), "S9-older");
    assert_open_pair(&smoke.hermes_rows(&pane), "S9", "S9-older");
    // The same session id again in a new process (`hermes --resume S9-older`) is
    // the daemon's Current: it is already checked in, so no context, no new
    // binding and no new seat.
    let again = smoke.drive(&pane, &[turn("S9-older", "T10")], true);
    assert_eq!(
        again.ack_mode(0).as_deref(),
        Some("current"),
        "{}",
        again.steps[0]
    );
    assert!(again.result(0).is_null(), "{}", again.steps[0]);
    assert_eq!(smoke.seat_ids(&pane), vec![seat]);
    assert_eq!(smoke.all_bindings(&pane), 2);
    assert_eq!(smoke.open_bindings(&pane), 2);

    // 11. A /background side agent (platform cli, new session, no parent) in a
    // seated pane: no new seat; it behaves like a launch-created seat (spec D2).
    let (pane, seat) = seated_pane(&mut smoke, "S11");
    let side = smoke.drive(
        &pane,
        &[session_start("side-S", Some("cli")), turn("side-S", "T1")],
        true,
    );
    assert_seated_turn(&side, 1, "startup");
    assert_eq!(smoke.seat_ids(&pane), vec![seat]);
    assert_eq!(smoke.all_bindings(&pane), 2);
    assert_eq!(
        smoke.open_bindings(&pane),
        2,
        "the side agent is its own binding in a new process; nothing ended S11 (spec decision 3, section 1)"
    );
    assert_eq!(smoke.open_session(&pane), "side-S");
    assert_open_pair(&smoke.hermes_rows(&pane), "S11", "side-S");
}

/// Two open bindings from two driver processes: the newer session is connected;
/// the earlier one stays open and was not superseded (different process, spec
/// section 3). Its liveness is timing-dependent (worker recheck), so not asserted.
fn assert_open_pair(rows: &[(String, String, Option<String>, String)], older: &str, newer: &str) {
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].0, older, "{rows:?}");
    assert_ne!(rows[0].2.as_deref(), Some("superseded"), "{rows:?}");
    assert_eq!(rows[1].0, newer, "{rows:?}");
    assert_eq!(rows[1].1, "connected", "{rows:?}");
}

#[test]
fn hermes_bare_enrollment_cold_daemon_configurations() {
    // No `daemon ensure` in setup: the hooks themselves must bring the daemon up.
    let Some(mut smoke) = Smoke::new("hermes_bare_enrollment_cold_daemon_configurations", false)
    else {
        return;
    };
    assert!(!smoke.daemon_up(), "setup started the daemon");

    // 8. The CLI start with the daemon down: the hook's `ensure_running` starts the
    // isolated daemon (inside the 4.5 s lifecycle cap) and enrolls the pane.
    let pane = smoke.pane();
    let started = smoke.drive(&pane, &[session_start("S8", Some("cli"))], true);
    assert!(started.result(0).is_null());
    assert!(
        smoke.daemon_up(),
        "the start hook did not bring the daemon up"
    );
    let seat = smoke.seat(&pane);
    assert_eq!(smoke.open_bindings(&pane), 0, "a start must not check in");
    let turn_after = smoke.drive(&pane, &[turn("S8", "T1")], true);
    assert_seated_turn(&turn_after, 0, "startup");
    assert_eq!(smoke.seat(&pane), seat);

    // 10. The start callback dropped (the bridge skipped it) with the daemon down: the
    // first top-level turn runs the existing `ensure_running`; a later turn
    // enrolls and checks in.
    smoke.ok(&["daemon", "stop"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while smoke.daemon_up() {
        assert!(Instant::now() < deadline, "daemon did not stop");
        std::thread::sleep(Duration::from_millis(100));
    }
    let pane = smoke.pane();
    let first = smoke.drive(&pane, &[turn("S10", "T1")], true);
    smoke.wait_daemon_up("first top-level turn");
    // The first turn runs `ensure_running` inside the 1.2 s turn cap. It either
    // completes the whole enrollment and check-in in that cap, or fails open
    // (null, or an unavailable notice) and leaves a daemon that is coming up;
    // both are the existing behavior, and either way the next turn ends with
    // exactly one seat and one open binding.
    let checked_in_cold = first.ack_mode(0).is_some();
    let later = smoke.drive(&pane, &[turn("S10", "T2")], true);
    if checked_in_cold {
        assert!(first.context(0).is_some(), "{}", first.steps[0]);
        assert_eq!(first.ack_mode(0).as_deref(), Some("startup"));
        assert_eq!(
            later.ack_mode(0).as_deref(),
            Some("current"),
            "{}",
            later.steps[0]
        );
    } else {
        assert_seated_turn(&later, 0, "startup");
    }
    smoke.seat(&pane);
    assert_eq!(smoke.open_bindings(&pane), 1);
    assert_eq!(smoke.all_bindings(&pane), 1);
}

fn no_herdr_path() -> &'static str {
    let path = "/usr/bin:/bin";
    assert!(
        !path.split(':').any(|d| Path::new(d).join("herdr").exists()),
        "herdr unexpectedly present in {path}"
    );
    path
}

/// The smoke follows the repo's isolated-Herdr skip policy: without herdr it
/// prints `skipped: no herdr: <test name>` under HT_SKIP_HERDR_TESTS=1 and fails
/// naming the binary otherwise, never a silent pass.
#[test]
fn smoke_skip_without_herdr_is_reported_by_test_name() {
    let path = no_herdr_path();
    let exe = std::env::current_exe().unwrap();
    let run = |skip: bool| {
        let mut cmd = herdr_threads::test_support::spawn::command(&exe);
        cmd.args([
            "--exact",
            "hermes_enrollment_smoke::smoke_installs_the_enrollment_bridge",
            "--nocapture",
        ])
        .env("PATH", path)
        .env_remove("HT_SKIP_HERDR_TESTS");
        if skip {
            cmd.env("HT_SKIP_HERDR_TESTS", "1");
        }
        cmd.output().unwrap()
    };
    let out = run(true);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("skipped: no herdr: smoke_installs_the_enrollment_bridge"),
        "{text}"
    );
    let out = run(false);
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "passed without herdr: {all}");
    assert!(all.contains("'herdr'"), "{all}");
}
