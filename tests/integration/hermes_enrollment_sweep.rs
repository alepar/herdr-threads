//! ht-4ne.5: integration sweep for Hermes global enrollment. The real binary and
//! detached daemon against an owned protocol stand-in (`FakeHost`); no Herdr, no
//! installed Hermes, no model. Envelopes are the ones the real bridge emits per
//! D2a row (`tests/fixtures/hermes/enrollment-envelopes.json`).
use herdr_threads::harness::adapter::EnrollmentPolicy;
use herdr_threads::test_support::{isolation::TestIsolation, spawn::SpawnOwned};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

/// A real daemon and canonical store behind an owned protocol stand-in.
struct Sweep {
    _host: super::sweep::FakeHost,
    isolation: TestIsolation,
    state: PathBuf,
    endpoint: PathBuf,
    sequence: u64,
}

impl Sweep {
    fn new() -> Self {
        let isolation = TestIsolation::new("hermes-enrollment-sweep");
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
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
        let host = super::sweep::FakeHost::start(
            &endpoint,
            (1..=3)
                .map(|n| super::sweep::pane(&format!("w1:p{n}"), &format!("sweep-term-{n}")))
                .collect(),
        );
        Self {
            isolation,
            state,
            endpoint,
            _host: host,
            sequence: 0,
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

    fn ensure_daemon(&self) {
        let output = self
            .command()
            .args(["--json", "daemon", "ensure"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "daemon ensure: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn hook(&self, pane: &str, harness: &str, payload: &Value) -> Value {
        let mut child = self
            .command()
            .args(["hook", harness])
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
            "{harness}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if output.stdout.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&output.stdout).unwrap()
        }
    }

    /// The real bridge's envelope for a D2a row with fresh timestamps and a
    /// strictly increasing observation sequence.
    fn envelope(&mut self, row: &str, session: &str, event: &str) -> Value {
        let rows: serde_json::Map<String, Value> = serde_json::from_slice(include_bytes!(
            "../fixtures/hermes/enrollment-envelopes.json"
        ))
        .unwrap();
        let mut payload = rows[row].clone();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        self.sequence += 1;
        payload["session_id"] = session.into();
        payload["event_id"] = event.into();
        if !payload["turn_id"].is_null() {
            payload["turn_id"] = event.into();
        }
        if payload["role_association"].is_object() {
            payload["role_association"]["session_id"] = session.into();
            if !payload["role_association"]["turn_id"].is_null() {
                payload["role_association"]["turn_id"] = event.into();
            }
        }
        payload["started_at"] = now.into();
        payload["deadline_at"] = (now + 1200).into();
        payload["observation_order"]["observed_at_millis"] = now.into();
        payload["observation_order"]["sequence"] = self.sequence.into();
        payload
    }

    /// A Hermes turn that must yield context. The envelope carries the production
    /// 1200 ms callback cap; a debug hook child on a loaded machine can overrun it
    /// and fail open with no context. Only such an overrun (the empty attempt
    /// itself took at least 1 s) is retried, the same event with fresh
    /// timestamps (a canonical replay), at most three more times.
    fn hermes_turn(&mut self, pane: &str, row: &str, session: &str, event: &str) -> Value {
        let mut output = Value::Null;
        for _ in 0..4 {
            let payload = self.envelope(row, session, event);
            let started = Instant::now();
            output = self.hook(pane, "hermes", &payload);
            if !output["context"].is_null() || started.elapsed() < Duration::from_secs(1) {
                break;
            }
        }
        output
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

    /// Resolved seats of one pane. The stand-in host names a pane's terminal
    /// `sweep-term-N`; the seat's target is whichever of the two the store keys on.
    fn resolved_seats(&self, pane: &str, terminal: &str) -> Vec<String> {
        let db = self.db();
        let mut statement = db
            .prepare(
                "SELECT id FROM seats WHERE state='resolved' AND (target_id=?1 OR target_id=?2) \
                 ORDER BY ordinal",
            )
            .unwrap();
        statement
            .query_map([pane, terminal], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn total_seats(&self) -> i64 {
        self.db()
            .query_row("SELECT COUNT(*) FROM seats", [], |row| row.get(0))
            .unwrap()
    }

    fn open_bindings(&self, harness: &str, seat: &str) -> i64 {
        self.db()
            .query_row(
                "SELECT COUNT(*) FROM occupant_bindings WHERE seat_id=?1 AND harness=?2 \
                 AND observation_provenance='cooperative_top_level' AND ended_at IS NULL",
                [seat, harness],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn open_session(&self, seat: &str) -> String {
        self.db()
            .query_row(
                "SELECT native_session FROM occupant_bindings WHERE seat_id=?1 \
                 AND harness='hermes' AND ended_at IS NULL",
                [seat],
                |row| row.get(0),
            )
            .unwrap()
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
            "seats",
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
}

impl Drop for Sweep {
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

/// The goal's main flow through the real binary: a bare CLI start enrolls (no
/// `seat resolve` anywhere), the first turn checks in with context, `/new` keeps
/// the seat, delegate children never enroll, and Claude/Codex still enroll on
/// their own start events.
#[test]
fn hermes_bare_cli_main_flow_enrolls_checks_in_keeps_seat_on_new_and_excludes_children() {
    let mut sweep = Sweep::new();
    sweep.ensure_daemon();
    let (pane, terminal) = ("w1:p1", "sweep-term-1");
    assert_eq!(sweep.total_seats(), 0, "no seat before the first event");

    // 1. CLI on_session_start: silent, one resolved seat, no binding yet.
    let start = sweep.envelope("cli_start", "sweep-s1", "start-s1");
    let out = sweep.hook(pane, "hermes", &start);
    assert!(out.is_null(), "a session start must print nothing: {out}");
    let seats = sweep.resolved_seats(pane, terminal);
    assert_eq!(seats.len(), 1, "start enrolled one seat: {seats:?}");
    let seat = seats[0].clone();
    assert_eq!(
        sweep.open_bindings("hermes", &seat),
        0,
        "enrollment alone binds nothing"
    );

    // 2. First top CLI turn: context plus a startup acknowledgement, one binding.
    let first = sweep.hermes_turn(pane, "top_turn_cli", "sweep-s1", "turn-s1");
    assert!(first["context"].is_string(), "no context: {first}");
    assert_eq!(first["lifecycle_ack"]["mode"], "startup", "{first}");
    assert_eq!(first["lifecycle_ack"]["session_id"], "sweep-s1");
    assert_eq!(sweep.open_bindings("hermes", &seat), 1);
    assert_eq!(sweep.open_session(&seat), "sweep-s1");
    assert_eq!(
        sweep.resolved_seats(pane, terminal),
        std::slice::from_ref(&seat)
    );

    // 3. /new: reset, a CLI start for the new session, its first turn. Same seat, clear.
    let reset = sweep.envelope("reset", "sweep-s2", "reset-s2");
    assert!(sweep.hook(pane, "hermes", &reset).is_null());
    let start = sweep.envelope("cli_start", "sweep-s2", "start-s2");
    sweep.hook(pane, "hermes", &start);
    let second = sweep.hermes_turn(pane, "top_turn_cli", "sweep-s2", "turn-s2");
    assert!(second["context"].is_string(), "no context: {second}");
    assert_eq!(second["lifecycle_ack"]["mode"], "clear", "{second}");
    assert_eq!(
        sweep.resolved_seats(pane, terminal),
        std::slice::from_ref(&seat),
        "/new moved or added a seat"
    );
    assert_eq!(sweep.open_bindings("hermes", &seat), 1);
    assert_eq!(sweep.open_session(&seat), "sweep-s2");

    // 4. Children never enroll and never change accountable state.
    let protected = sweep.accountable_snapshot();
    let seats_before = sweep.total_seats();
    let mut child = sweep.envelope("top_turn_cli", "sweep-child", "turn-child");
    child["parent_session_id"] = "sweep-s2".into();
    child["shape"]["parent_session_id"] = json!({"presence":"present","type":"string"});
    child["role_association"]["role"] = "child".into();
    child["role_association"]["enrollment_witness"] = Value::Null;
    let out = sweep.hook(pane, "hermes", &child);
    assert!(out["lifecycle_ack"].is_null(), "child checked in: {out}");
    let mut subagent_start = sweep.envelope("empty_platform_start", "sweep-child", "start-child");
    subagent_start["platform"] = "subagent".into();
    assert!(sweep.hook(pane, "hermes", &subagent_start).is_null());
    // A child in a pane that has no seat at all enrolls nothing either.
    let out = sweep.hook("w1:p3", "hermes", &child);
    assert!(out["lifecycle_ack"].is_null(), "child checked in: {out}");
    assert!(
        sweep.hook("w1:p3", "hermes", &subagent_start).is_null(),
        "subagent start printed"
    );
    assert_eq!(
        sweep.total_seats(),
        seats_before,
        "a child allocated a seat"
    );
    assert_eq!(
        sweep.accountable_snapshot(),
        protected,
        "a child changed accountable rows"
    );

    // 5. Claude and Codex are unchanged: their start event enrolls and checks in.
    let claude = json!({"hook_event_name": "SessionStart", "source": "startup", "session_id": "sweep-claude"});
    let codex = json!({"hook_event_name": "SessionStart", "source": "startup", "session_id": "sweep-codex", "turn_id": "sweep-turn"});
    for (pane, terminal, harness, payload) in [
        ("w1:p2", "sweep-term-2", "claude", claude),
        ("w1:p3", "sweep-term-3", "codex", codex),
    ] {
        let out = sweep.hook(pane, harness, &payload);
        assert!(
            !out["hookSpecificOutput"].is_null(),
            "{harness}: no context: {out}"
        );
        let seats = sweep.resolved_seats(pane, terminal);
        assert_eq!(seats.len(), 1, "{harness}: {seats:?}");
        assert_eq!(sweep.open_bindings(harness, &seats[0]), 1, "{harness}");
    }
    // Hermes' seat is untouched by the other adapters.
    assert_eq!(sweep.open_bindings("hermes", &seat), 1);
    assert_eq!(sweep.open_session(&seat), "sweep-s2");
}

/// Every registered adapter declares its enrollment rule, and the hook has no
/// hard-coded startup rule left: enrollment is decided by `DecodedEvent::enrollment`.
#[test]
fn every_registered_adapter_declares_its_enrollment_policy() {
    let expected = |id: &str| match id {
        "claude" | "codex" | "synthetic_fourth" => EnrollmentPolicy::Lifecycle,
        "hermes" => EnrollmentPolicy::WitnessedStartAndUnownedTurn,
        other => panic!("adapter {other} has no row in the enrollment policy table"),
    };
    let registry = herdr_threads::harness::registry::builtins();
    let mut seen = Vec::new();
    for registration in registry.registrations() {
        let id = registration.metadata().id;
        assert_eq!(registration.enrollment_policy(), expected(id), "{id}");
        seen.push(id);
    }
    seen.sort_unstable();
    assert_eq!(seen, ["claude", "codex", "hermes", "synthetic_fourth"]);
    let hook = include_str!("../../src/cli/hook.rs");
    assert!(
        !hook.contains(
            "EventIntent::Lifecycle(EventKind::Startup | EventKind::Clear | EventKind::Resume)"
        ),
        "src/cli/hook.rs still hard-codes the lifecycle enrollment rule"
    );
}
