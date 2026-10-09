//! ht-p03.21: configuration smoke. Each Herdr up/down state runs once through
//! `daemon ensure`, `daemon health --json` and `doctor --json` against an
//! isolated named Herdr session (never the shared server). The tests assert
//! only that every command completes with a recorded exit code; the outcome
//! classification lives in
//! `docs/history/remaining-findings-run/config-smoke.md`.
//! Set `HT_CONFIG_SMOKE_OUT` to capture `<state>/<step>.{out,err,code}`.
use herdr_threads::test_support::isolated_herdr::{HerdrState, IsolatedHerdr};
use std::{
    fs,
    path::{Path, PathBuf},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Smoke {
    herdr: IsolatedHerdr,
    state: &'static str,
    plugin_state: PathBuf,
}

impl Smoke {
    fn new(state: &'static str) -> Option<Self> {
        let herdr = IsolatedHerdr::new(state)?;
        let plugin_state = herdr.root().join("plugin-state");
        fs::create_dir_all(&plugin_state).unwrap();
        Some(Self {
            herdr,
            state,
            plugin_state,
        })
    }

    fn cli(&self, args: &[&str]) -> (i32, String, String) {
        let output = self
            .herdr
            .command(BIN)
            .arg("--json")
            .arg("--state-dir")
            .arg(&self.plugin_state)
            .arg("--host-endpoint")
            .arg(self.herdr.socket_path())
            .args(args)
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("HERDR_ENV")
            .output()
            .unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// Run one step, print and (when asked) record its output, return the exit code.
    fn step(&self, name: &str, args: &[&str]) -> i32 {
        let (code, out, err) = self.cli(args);
        println!(
            "[{}] {name}: exit={code}\n  stdout: {}\n  stderr: {}",
            self.state,
            out.trim(),
            err.trim()
        );
        if let Some(dir) = std::env::var_os("HT_CONFIG_SMOKE_OUT") {
            let dir = Path::new(&dir).join(self.state);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(format!("{name}.out")), &out).unwrap();
            fs::write(dir.join(format!("{name}.err")), &err).unwrap();
            fs::write(dir.join(format!("{name}.code")), format!("{code}\n")).unwrap();
        }
        assert_ne!(code, -1, "{name} was killed by a signal");
        code
    }

    fn triple(&self, label: &str) {
        self.step(&format!("{label}-ensure"), &["daemon", "ensure"]);
        self.step(&format!("{label}-health"), &["daemon", "health"]);
        self.step(&format!("{label}-doctor"), &["doctor"]);
    }
}

impl Drop for Smoke {
    fn drop(&mut self) {
        // Every daemon a smoke test starts is stopped before the test ends.
        let _ = self.cli(&["daemon", "stop"]);
    }
}

#[test]
fn herdr_up_linked() {
    let Some(s) = Smoke::new("herdr_up_linked") else {
        return;
    };
    s.herdr.start();
    assert_eq!(s.herdr.state(), HerdrState::Up);
    s.triple("up");
}

#[test]
fn herdr_stopped_while_daemon_runs() {
    let Some(s) = Smoke::new("herdr_stopped_while_daemon_runs") else {
        return;
    };
    s.herdr.start();
    s.step("before-ensure", &["daemon", "ensure"]);
    s.herdr.stop();
    assert_eq!(s.herdr.state(), HerdrState::Stopped);
    s.step("stopped-health", &["daemon", "health"]);
    s.step("stopped-doctor", &["doctor"]);
}

#[test]
fn herdr_never_started() {
    let Some(s) = Smoke::new("herdr_never_started") else {
        return;
    };
    assert_eq!(s.herdr.state(), HerdrState::NeverStarted);
    assert!(!s.herdr.socket_path().exists());
    s.triple("never");
}

#[test]
fn herdr_stale_socket_dead_server() {
    let Some(s) = Smoke::new("herdr_stale_socket_dead_server") else {
        return;
    };
    s.herdr.stale_socket();
    assert_eq!(s.herdr.state(), HerdrState::StaleSocket);
    s.triple("stale");
}

#[test]
fn herdr_restarted_under_running_daemon() {
    let Some(s) = Smoke::new("herdr_restarted_under_running_daemon") else {
        return;
    };
    s.herdr.start();
    s.step("before-ensure", &["daemon", "ensure"]);
    s.herdr.restart();
    assert_eq!(s.herdr.state(), HerdrState::Up);
    assert_eq!(s.herdr.starts(), 2);
    s.step("restarted-health", &["daemon", "health"]);
    s.step("restarted-doctor", &["doctor"]);
}

/// This fixture exercises the real detached daemon and canonical store against
/// an owned protocol stand-in. It never starts an installed Herdr/Hermes.
struct SyntheticDaemon {
    _host: super::sweep::FakeHost,
    isolation: herdr_threads::test_support::isolation::TestIsolation,
    state: PathBuf,
    endpoint: PathBuf,
}
impl SyntheticDaemon {
    fn new() -> Self {
        let isolation =
            herdr_threads::test_support::isolation::TestIsolation::new("adapter-config-smoke");
        let state = isolation.path("state");
        let endpoint = isolation.socket_path("host.sock");
        let runtime = herdr_threads::daemon::paths::RuntimeContext::explicit(
            state.clone(),
            endpoint.clone(),
            None,
        )
        .unwrap();
        let paths = herdr_threads::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
        paths.prepare_instance_dir().unwrap();
        let settings = paths
            .instance_dir
            .join(herdr_threads::daemon::settings::SETTINGS_FILE);
        fs::write(&settings, br#"{"harness_manifest":"off"}"#).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
        let host = super::sweep::FakeHost::start(
            &endpoint,
            (1..=4)
                .map(|n| super::sweep::pane(&format!("w1:p{n}"), &format!("synthetic-term-{n}")))
                .collect(),
        );
        Self {
            isolation,
            state,
            endpoint,
            _host: host,
        }
    }
    fn command(&self) -> std::process::Command {
        let mut command = self.isolation.command(BIN);
        command
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.endpoint)
            .env("CLAUDE_CONFIG_DIR", self.isolation.path("home/claude"))
            .env("CODEX_HOME", self.isolation.path("home/codex"))
            .env("HERMES_HOME", self.isolation.path("home/hermes"));
        command
    }
    fn json(&self, args: &[&str]) -> serde_json::Value {
        let output = self.command().arg("--json").args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn hook(&self, pane: &str, harness: &str, payload: &serde_json::Value) -> serde_json::Value {
        use herdr_threads::test_support::spawn::SpawnOwned;
        use std::io::Write;
        let mut child = self
            .command()
            .args(["hook", harness])
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", pane)
            .env("HERDR_SOCKET_PATH", &self.endpoint)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
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
            "{harness}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if output.stdout.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&output.stdout).unwrap()
        }
    }
    fn db(&self) -> rusqlite::Connection {
        let runtime = herdr_threads::daemon::paths::RuntimeContext::explicit(
            self.state.clone(),
            self.endpoint.clone(),
            None,
        )
        .unwrap();
        let paths = herdr_threads::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
        rusqlite::Connection::open(paths.database_path).unwrap()
    }
    fn accountable_snapshot(&self) -> Vec<Vec<String>> {
        let db = self.db();
        [
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
            let mut statement = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let columns = statement.column_count();
            let mut rows = statement
                .query_map([], |row| {
                    let values = (0..columns)
                        .map(|column| row.get::<_, rusqlite::types::Value>(column))
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok(format!("{values:?}"))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort();
            rows
        })
        .into()
    }
    /// Accountable row counts. `operations` is left out: lifecycle startup
    /// enrollment records a fresh guarded resolution per event, which reuses
    /// the resolved seat and allocates nothing.
    fn counts(&self) -> [i64; 8] {
        let db = self.db();
        [
            "occupant_bindings",
            "receipts",
            "invitations",
            "warning_offer",
            "digest_notice_offer",
            "receipt_state",
            "delivery_observations",
            "seat_availability",
        ]
        .map(|table| {
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        })
    }
}
impl Drop for SyntheticDaemon {
    fn drop(&mut self) {
        let output = self.command().args(["daemon", "stop"]).output();
        if !std::thread::panicking() {
            assert!(
                output.is_ok_and(|out| out.status.success()),
                "owned daemon did not stop"
            );
        }
    }
}

/// A Hermes `pre_llm_call` that must yield context. Its envelope carries the
/// production 1200 ms callback cap; a debug hook child on a loaded machine can
/// overrun it and fail open with no context. Only such an overrun (the empty
/// attempt itself took at least 1 s) is retried: the same event is presented
/// again with fresh timestamps (a canonical replay; the caller asserts the
/// binding did not rotate), at most three more times. A fast empty answer is
/// returned as is, so a callback that never yields context still fails.
fn hermes_hook_with_context(
    daemon: &SyntheticDaemon,
    pane: &str,
    session: &str,
    event: &str,
    sequence: u64,
) -> serde_json::Value {
    let mut output = serde_json::Value::Null;
    for _ in 0..4 {
        let started = std::time::Instant::now();
        output = daemon.hook(pane, "hermes", &hermes_envelope(session, event, sequence));
        if !output["context"].is_null() || started.elapsed() < std::time::Duration::from_secs(1) {
            break;
        }
    }
    output
}

fn hermes_envelope(session: &str, event: &str, sequence: u64) -> serde_json::Value {
    let mut payload: serde_json::Value =
        serde_json::from_slice(include_bytes!("../fixtures/hermes/envelopes.json")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    payload["session_id"] = session.into();
    payload["event_id"] = event.into();
    payload["turn_id"] = event.into();
    payload["role_association"]["session_id"] = session.into();
    payload["role_association"]["turn_id"] = event.into();
    payload["started_at"] = now.into();
    payload["deadline_at"] = (now + 1200).into();
    payload["observation_order"]["observed_at_millis"] = now.into();
    payload["observation_order"]["sequence"] = sequence.into();
    payload
}

/// Kills parser-only author proofs: real hook children must durably check in,
/// replay without binding rotation, and suppress every non-top-level input.
#[test]
fn synthetic_four_adapter_hook_configuration_and_canonical_replay() {
    use serde_json::json;
    let daemon = SyntheticDaemon::new();
    daemon.json(&["daemon", "ensure"]);
    let mut seats = Vec::new();
    for n in 1..=4 {
        let value = daemon.json(&["seat", "resolve", "--pane", &format!("w1:p{n}")]);
        seats.push(value["result"]["data"].as_str().unwrap().to_owned());
    }
    let claude = json!({"hook_event_name": "SessionStart", "source": "startup", "session_id": "synthetic-claude"});
    let codex = json!({"hook_event_name": "SessionStart", "source": "startup", "session_id": "synthetic-codex", "turn_id": "synthetic-turn"});
    let fourth = json!({"event": "SyntheticStart", "event_id": "synthetic-start", "session_id": "synthetic-fourth-session", "role": "top_level"});
    for (pane, harness, payload, key) in [
        ("w1:p1", "claude", claude, "hookSpecificOutput"),
        ("w1:p2", "codex", codex, "hookSpecificOutput"),
        (
            "w1:p3",
            "hermes",
            hermes_envelope("synthetic-hermes", "hermes-first", 1),
            "context",
        ),
        (
            "w1:p4",
            "synthetic_fourth",
            fourth.clone(),
            "synthetic_context",
        ),
    ] {
        let output = if harness == "hermes" {
            hermes_hook_with_context(&daemon, pane, "synthetic-hermes", "hermes-first", 1)
        } else {
            daemon.hook(pane, harness, &payload)
        };
        assert!(
            !output[key].is_null(),
            "{harness}: real callback produced no context: {output}"
        );
    }
    let bindings = daemon.db().prepare("SELECT harness,observation_provenance FROM occupant_bindings WHERE ended_at IS NULL ORDER BY harness").unwrap()
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(
        bindings,
        ["claude", "codex", "hermes", "synthetic_fourth"]
            .map(|id| (id.to_owned(), "cooperative_top_level".to_owned()))
    );
    let before = daemon.counts();
    let operations_before: i64 = daemon
        .db()
        .query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM operations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let first = daemon.hook("w1:p4", "synthetic_fourth", &fourth);
    let replay = daemon.hook("w1:p4", "synthetic_fourth", &fourth);
    assert_eq!(first, replay);
    assert_eq!(
        daemon.counts(),
        before,
        "exact replay allocated canonical rows"
    );
    // The only operations a replay adds are guarded resolutions of the same
    // already resolved seat: nothing else is decided again.
    let added: Vec<String> = daemon
        .db()
        .prepare("SELECT result_json FROM operations WHERE rowid > ?1")
        .unwrap()
        .query_map([operations_before], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let resolved: std::collections::BTreeSet<String> = added
        .iter()
        .map(|json| {
            let value: serde_json::Value = serde_json::from_str(json).unwrap();
            assert_eq!(
                value["kind"], "seat_resolved",
                "replay decided more: {json}"
            );
            value["data"].as_str().unwrap().to_owned()
        })
        .collect();
    assert!(
        resolved.len() <= 1,
        "replay resolved different seats: {resolved:?}"
    );
    daemon.json(&["daemon", "stop"]);
    daemon.json(&["daemon", "ensure"]);
    assert_eq!(
        daemon.hook("w1:p4", "synthetic_fourth", &fourth),
        replay,
        "canonical replay changed after process restart"
    );
    let after_restart = daemon.counts();
    assert_eq!(
        &after_restart[..7],
        &before[..7],
        "restart replay allocated accountable rows"
    );
    let before = after_restart;
    let protected = daemon.accountable_snapshot();
    for role in ["child", "unknown"] {
        let mut payload = fourth.clone();
        payload["role"] = role.into();
        payload["event_id"] = format!("synthetic-{role}").into();
        daemon.hook("w1:p4", "synthetic_fourth", &payload);
        assert_eq!(
            daemon.accountable_snapshot(),
            protected,
            "fourth {role} changed accountable rows"
        );
        assert_eq!(
            daemon.counts(),
            before,
            "fourth {role} performed accountable work"
        );
    }
    let observer = json!({"event": "SyntheticObserver", "event_id": "synthetic-observer", "session_id": "synthetic-fourth-session", "role": "top_level"});
    assert!(daemon.hook("w1:p4", "synthetic_fourth", &observer)["synthetic_context"].is_null());
    assert_eq!(daemon.counts(), before);
    assert_eq!(
        daemon.accountable_snapshot(),
        protected,
        "fourth observer changed accountable rows"
    );
    for (parent, role) in [(Some("parent"), "child"), (None, "unknown")] {
        let mut payload = hermes_envelope("synthetic-hermes", &format!("hermes-{role}"), 2);
        if let Some(parent) = parent {
            payload["parent_session_id"] = parent.into();
            payload["role_association"]["role"] = "child".into();
        } else {
            payload["parent_session_id"] = serde_json::Value::Null;
            payload["role_association"]["role"] = "unknown".into();
            payload["shape"]["parent_session_id"] = json!({"presence":"missing", "type":"absent"});
        }
        let output = daemon.hook("w1:p3", "hermes", &payload);
        assert!(output["lifecycle_ack"].is_null());
        assert_eq!(
            daemon.accountable_snapshot(),
            protected,
            "Hermes {role} changed accountable rows"
        );
        assert_eq!(
            daemon.counts(),
            before,
            "Hermes {role} performed accountable work"
        );
    }
    let mut post = hermes_envelope("synthetic-hermes", "hermes-post", 3);
    post["callback"] = "post_tool_call".into();
    post["turn_id"] = "hermes-first".into();
    post["role_association"]["turn_id"] = "hermes-first".into();
    post["role_association"]["provenance"] = "qualified_pre_llm_cache".into();
    post["parent_session_id"] = serde_json::Value::Null;
    post["shape"]["parent_session_id"] = json!({"presence":"missing", "type":"absent"});
    assert!(daemon.hook("w1:p3", "hermes", &post)["lifecycle_ack"].is_null());
    assert_eq!(
        daemon.counts(),
        before,
        "qualified post-tool observer performed accountable work"
    );
    assert_eq!(
        daemon.accountable_snapshot(),
        protected,
        "post-tool observer changed accountable rows"
    );
    let mut reset = hermes_envelope("synthetic-hermes", "hermes-reset", 4);
    reset["callback"] = "on_session_reset".into();
    reset["reset_reason"] = "new_session".into();
    reset["role_association"] = serde_json::Value::Null;
    assert!(daemon.hook("w1:p3", "hermes", &reset).is_null());
    assert_eq!(
        daemon.counts(),
        before,
        "declared reset changed canonical binding before qualified turn"
    );
    assert_eq!(
        daemon.accountable_snapshot(),
        protected,
        "reset observer changed accountable rows"
    );
    let next = hermes_hook_with_context(&daemon, "w1:p3", "synthetic-hermes-new", "hermes-next", 5);
    assert!(next["context"].is_string());
    assert_eq!(
        daemon
            .db()
            .query_row(
                "SELECT COUNT(*) FROM occupant_bindings WHERE harness='hermes'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    let health = daemon.json(&["daemon", "health"]);
    assert_eq!(
        health["result"]["data"]["settings"]["wake_batch_delay_ms"],
        0
    );
    let doctor = daemon.json(&["doctor"]);
    assert_eq!(doctor["doctor"]["harness_manifest"]["policy"], "off");
    assert!(doctor.to_string().contains("synthetic_fourth"));
    assert!(!doctor.to_string().contains("runtime verified"));
    assert_eq!(
        seats.iter().collect::<std::collections::HashSet<_>>().len(),
        4
    );
}
