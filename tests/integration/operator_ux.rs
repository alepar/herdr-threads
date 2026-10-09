//! A person's own pane identity and `--pane` names through the installed
//! executable, its detached daemon and the private Herdr endpoint of
//! `sweep.rs`. The stand-in agent seat uses the cooperative caller flags; the
//! person uses only `HERDR_PANE_ID`, as a shell inside Herdr would.

use super::sweep::{FakeHost, Scratch, agent_pane, pane};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Plugin {
    state: PathBuf,
    host: PathBuf,
}
impl Plugin {
    /// `pane`: the invoking shell's `HERDR_PANE_ID`; `agent`: cooperative
    /// flags `(seat, pane)` for the stand-in Claude top-level agent.
    fn run(
        &self,
        pane: Option<&str>,
        agent: Option<(&str, &str)>,
        args: &[&str],
    ) -> (i32, Value, String) {
        self.run_with_env(pane, agent, args, &[])
    }
    /// `run` with extra environment variables (the agent-marker tests).
    fn run_with_env(
        &self,
        pane: Option<&str>,
        agent: Option<(&str, &str)>,
        args: &[&str],
        extra_env: &[(&str, &str)],
    ) -> (i32, Value, String) {
        let mut command = crate::scrubbed_command(BIN);
        let args = if args.first() == Some(&"human") {
            command.arg("human");
            &args[1..]
        } else {
            args
        };
        command
            .arg("--json")
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.host);
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
        // The runner's own agent environment must not reach the guard under test.
        for (key, _) in std::env::vars() {
            if key.starts_with("CODEX_") {
                command.env_remove(key);
            }
        }
        command
            .args(args)
            .env_remove("CLAUDECODE")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("HERDR_ENV");
        if let Some(pane) = pane {
            command.env("HERDR_PANE_ID", pane);
        }
        command.envs(extra_env.iter().copied());
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let value = serde_json::from_str(&stdout).unwrap_or(Value::Null);
        (output.status.code().unwrap_or(-1), value, stderr)
    }
    fn ok(&self, pane: Option<&str>, agent: Option<(&str, &str)>, args: &[&str]) -> Value {
        let (code, value, stderr) = self.run(pane, agent, args);
        assert_eq!(code, 0, "herdr-threads {args:?} failed: {stderr}{value}");
        value["result"].clone()
    }
    fn refused(&self, pane: Option<&str>, args: &[&str]) -> String {
        self.refused_with_env(pane, args, &[])
    }
    fn refused_with_env(
        &self,
        pane: Option<&str>,
        args: &[&str],
        extra_env: &[(&str, &str)],
    ) -> String {
        let (code, value, stderr) = self.run_with_env(pane, None, args, extra_env);
        assert_eq!(
            code, 2,
            "herdr-threads {args:?} must be refused as invalid: {stderr}{value}"
        );
        stderr
    }
    fn operator_mark_files(&self) -> usize {
        fn walk(dir: &Path) -> usize {
            fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| {
                    let path = entry.path();
                    if path.is_dir() {
                        walk(&path)
                    } else {
                        usize::from(path.file_name().is_some_and(|n| n == "operator.json"))
                    }
                })
                .sum()
        }
        walk(&self.state)
    }
    fn database(&self) -> rusqlite::Connection {
        fn find(dir: &Path) -> Option<PathBuf> {
            for entry in fs::read_dir(dir).ok()?.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .is_some_and(|name| name == "threads.sqlite3")
                {
                    return Some(path);
                }
                if path.is_dir()
                    && let Some(found) = find(&path)
                {
                    return Some(found);
                }
            }
            None
        }
        let path = find(&self.state).expect("daemon database");
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap()
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.run(None, None, &["daemon", "stop"]);
    }
}

fn labeled(id: &str, terminal: &str, label: &str) -> Value {
    let mut value = pane(id, terminal);
    value["label"] = json!(label);
    value
}

fn provenance(observation: Option<String>) -> String {
    let observation = observation.expect("recorded observation");
    let value: Value = serde_json::from_str(&observation).unwrap();
    value["provenance"].as_str().unwrap().to_owned()
}

/// `me init` → `thread create` → `invite` an agent seat → `send
/// --require-ack` from the person's pane with no caller flags; the person's
/// binding, send, ACK and acceptance are recorded as `operator_human`, while
/// the agent's own ACK stays its unchanged `cooperative_top_level` claim.
/// `--pane` resolves a unique Herdr pane label and refuses ambiguous and
/// unknown names with how to find the ID.
#[test]
fn person_pane_identity_sends_and_acks_without_flags_with_operator_provenance() {
    let root = std::env::temp_dir().join(format!(
        "htme-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let _scratch = Scratch(root.clone());
    let socket = root.join("herdr.sock");
    let mut agent_pane = pane("w1:p5", "term-e");
    agent_pane["agent"] = json!("claude");
    let _host = FakeHost::start(
        &socket,
        vec![
            labeled("w1:p1", "term-a", "me-pane"),
            labeled("w1:p2", "term-b", "try-target"),
            labeled("w1:p3", "term-c", "dup"),
            labeled("w1:p4", "term-d", "dup"),
            agent_pane,
        ],
    );
    let plugin = Plugin {
        state: root.join("state"),
        host: socket.clone(),
    };
    plugin.ok(None, None, &["daemon", "ensure"]);

    // (b) --pane accepts a unique pane label and resolves it to the pane ID.
    let agent = plugin.ok(
        None,
        None,
        &[
            "seat",
            "resolve",
            "--space",
            "w1",
            "--tab",
            "w1:t1",
            "--pane",
            "try-target",
        ],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let by_id = plugin.ok(None, None, &["seat", "resolve", "--pane", "w1:p2"]);
    assert_eq!(
        by_id["data"],
        agent.as_str(),
        "name and ID name the same pane"
    );
    let (code, _, ambiguous) = plugin.run(
        None,
        None,
        &[
            "seat", "resolve", "--space", "w1", "--tab", "w1:t1", "--pane", "dup",
        ],
    );
    assert_eq!(code, 1, "an ambiguous locator is a conflict: {ambiguous}");
    assert!(ambiguous.contains("(conflict)"), "{ambiguous}");
    assert!(ambiguous.contains("ambiguous"), "{ambiguous}");
    assert!(
        ambiguous.contains("w1:p3") && ambiguous.contains("w1:p4"),
        "{ambiguous}"
    );
    assert!(ambiguous.contains("herdr pane current"), "{ambiguous}");
    let (code, _, missing) = plugin.run(
        None,
        None,
        &[
            "seat", "resolve", "--space", "w1", "--tab", "w1:t1", "--pane", "nope",
        ],
    );
    assert_eq!(code, 1, "an unknown pane is not found: {missing}");
    assert!(missing.contains("\"nope\""), "{missing}");
    assert!(missing.contains("herdr pane current"), "{missing}");

    // The stand-in agent registers as it would from its hook.
    let agent_caller = Some((agent.as_str(), "w1:p2"));
    plugin.ok(
        None,
        agent_caller,
        &["check-in", "--lifecycle-event", "agent-start"],
    );

    // (a) Without `me init` the person's pane has no caller identity.
    let before = plugin.refused(
        Some("w1:p1"),
        &["human", "thread", "create", "--topic", "too early"],
    );
    assert!(before.contains("me init"), "{before}");

    // Before the human check-in, an unbound seat can inherit an agent obligation.
    let person_before = plugin.ok(None, None, &["seat", "resolve", "--pane", "w1:p1"])["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let legacy_thread = plugin.ok(
        None,
        agent_caller,
        &["thread", "create", "--topic", "older obligation"],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    plugin.ok(
        None,
        agent_caller,
        &["invite", &legacy_thread, "--seat", &person_before],
    );
    let legacy = plugin.ok(
        None,
        agent_caller,
        &[
            "send",
            &legacy_thread,
            "--body",
            "older addressed mail",
            "--require-ack",
            &person_before,
        ],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let legacy_pending = plugin.ok(None, None, &["pending-receipts", "--seat", &person_before]);
    assert!(
        legacy_pending.to_string().contains(&legacy),
        "{legacy_pending}"
    );

    // `me init` resolves the pane by ordinary resolution and checks in as a person.
    let me = plugin.ok(Some("w1:p1"), None, &["human", "me", "init"]);
    assert_eq!(me["kind"], "checked_in", "{me}");
    assert_eq!(me["data"]["context"]["harness"], "human", "{me}");
    assert_eq!(me["data"]["context"]["role"], "top_level", "{me}");
    let person = me["data"]["context"]["seat"].as_str().unwrap().to_owned();
    assert_ne!(person, agent);
    assert_eq!(person, person_before);
    let waived = plugin.ok(None, None, &["pending-receipts", "--seat", &person]);
    assert!(
        !waived.to_string().contains(&legacy),
        "human check-in waives the old obligation: {waived}"
    );
    // Re-running is a current check-in for the same occupant.
    let again = plugin.ok(Some("w1:p1"), None, &["human", "me", "init"]);
    assert_eq!(again["data"]["context"], me["data"]["context"], "{again}");

    // From the person's pane, with no flags: create, invite, send --require-ack, read.
    let thread = plugin.ok(
        Some("w1:p1"),
        None,
        &["human", "thread", "create", "--topic", "human handoff"],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    plugin.ok(
        Some("w1:p1"),
        None,
        &["human", "invite", &thread, "--seat", &agent],
    );
    let handoff = plugin.ok(
        Some("w1:p1"),
        None,
        &[
            "human",
            "send",
            &thread,
            "--body",
            "please review",
            "--require-ack",
            &agent,
        ],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let history = plugin.ok(Some("w1:p1"), None, &["human", "read", &thread]);
    assert!(history.to_string().contains(&handoff), "{history}");
    let pending = plugin.ok(None, None, &["pending-receipts", "--seat", &agent]);
    assert!(pending.to_string().contains(&handoff), "{pending}");

    // The agent's own receipt semantics are unchanged: it ACKs and accepts.
    plugin.ok(None, agent_caller, &["ack", &handoff]);
    plugin.ok(None, agent_caller, &["accept", &thread]);

    // Mail to the human stays readable without creating an ACK obligation.
    let reply = plugin.ok(
        None,
        agent_caller,
        &[
            "send",
            &thread,
            "--body",
            "done, please confirm",
            "--require-ack",
            &person,
        ],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let inbox = plugin.ok(Some("w1:p1"), None, &["human", "inbox"]);
    assert!(
        !inbox.to_string().contains(&thread),
        "no human receipt expectation: {inbox}"
    );
    let history = plugin.ok(Some("w1:p1"), None, &["human", "read", &thread]);
    assert!(history.to_string().contains(&reply), "{history}");
    let pending = plugin.ok(None, None, &["pending-receipts", "--seat", &person]);
    assert!(!pending.to_string().contains(&reply), "{pending}");
    let (code, _, refused) = plugin.run(Some("w1:p1"), None, &["human", "ack", &reply]);
    assert_eq!(
        code, 2,
        "human-only delivery has no retained receipt to ACK: {refused}"
    );
    assert!(
        refused.contains("not an addressed ordinary message"),
        "{refused}"
    );
    // The person may still explicitly ACK retained older mail, with human provenance.
    let acked = plugin.ok(Some("w1:p1"), None, &["human", "ack", &legacy]);
    assert_eq!(acked["data"]["acknowledged"], json!([legacy]), "{acked}");

    // The person accepts an agent's invitation from their pane.
    let second = plugin.ok(
        None,
        agent_caller,
        &["thread", "create", "--topic", "agent asks"],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    plugin.ok(None, agent_caller, &["invite", &second, "--seat", &person]);
    assert_eq!(
        plugin.ok(Some("w1:p1"), None, &["human", "accept", &second])["kind"],
        "accepted"
    );

    // Provenance: the person is operator_human everywhere; the agent is not.
    let db = plugin.database();
    let (harness, binding): (String, String) = db
        .query_row(
            "SELECT harness,observation_provenance FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
            [&person],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (harness.as_str(), binding.as_str()),
        ("human", "operator_human")
    );
    let availability: String = db
        .query_row(
            "SELECT observation_provenance FROM seat_availability WHERE seat_id=?1 ORDER BY decision_seq DESC LIMIT 1",
            [&person],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(availability, "operator_human");
    let sent: Option<String> = db
        .query_row(
            "SELECT native_observation FROM messages WHERE id=?1",
            [&handoff],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(provenance(sent), "operator_human");
    let ack = |message: &str, seat: &str| -> Option<String> {
        db.query_row(
            "SELECT COALESCE((SELECT ack_observation FROM receipt_state WHERE message_id=?1 AND seat_id=?2),\
                             (SELECT ack_observation FROM receipts WHERE message_id=?1 AND seat_id=?2))",
            [message, seat],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(provenance(ack(&legacy, &person)), "operator_human");
    assert_eq!(provenance(ack(&handoff, &agent)), "cooperative_top_level");
    let accepted: Option<String> = db
        .query_row(
            "SELECT accepted_observation FROM invitations WHERE thread_id=?1 AND seat_id=?2 AND state='accepted'",
            [&second, &person],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(provenance(accepted), "operator_human");

    // `me init` never acts as an agent and needs the invoking pane.
    let in_agent_pane = plugin.refused(Some("w1:p5"), &["human", "me", "init"]);
    assert!(in_agent_pane.contains("agent"), "{in_agent_pane}");
    let no_pane = plugin.refused(None, &["human", "me", "init"]);
    assert!(no_pane.contains("HERDR_PANE_ID"), "{no_pane}");
    // Nor does it take over an agent's registered seat.
    let agent_seat = plugin.refused(Some("w1:p2"), &["human", "me", "init"]);
    assert!(agent_seat.contains("never takes over"), "{agent_seat}");

    // An agent started later in the person's pane replaces the person at its
    // lifecycle check-in (new generation); the person's history keeps its
    // operator_human provenance and `me init` there is then refused.
    drop(db);
    let launched = plugin.ok(
        None,
        Some((person.as_str(), "w1:p1")),
        &["check-in", "--lifecycle-event", "agent-in-person-pane"],
    );
    assert_eq!(
        launched["data"]["context"]["harness"], "claude",
        "{launched}"
    );
    let db = plugin.database();
    let bindings: Vec<(String, String, bool)> = db
        .prepare("SELECT harness,observation_provenance,ended_at IS NULL FROM occupant_bindings WHERE seat_id=?1 ORDER BY ordinal")
        .unwrap()
        .query_map([&person], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        bindings,
        vec![
            ("human".to_owned(), "operator_human".to_owned(), false),
            (
                "claude".to_owned(),
                "cooperative_top_level".to_owned(),
                true
            ),
        ]
    );
    let replaced = plugin.refused(Some("w1:p1"), &["human", "me", "init"]);
    assert!(replaced.contains("never takes over"), "{replaced}");
}

fn scratch(prefix: &str) -> (PathBuf, Scratch, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let guard = Scratch(root.clone());
    let socket = root.join("herdr.sock");
    (root, guard, socket)
}

const OVERRIDE_ARGV: &str = "herdr-threads me init --operator";
const HUMAN_OVERRIDE_ARGV: &str = "herdr-threads human me init --operator";

/// TRUST-POLICY A4, client side: an agent environment marker refuses `me init`
/// and names the override; `--operator` is the explicit escape.
#[test]
fn me_init_refuses_with_claudecode_or_codex_marker() {
    let (root, _scratch, socket) = scratch("htmk");
    let _host = FakeHost::start(&socket, vec![pane("w1:p1", "term-a")]);
    let plugin = Plugin {
        state: root.join("state"),
        host: socket.clone(),
    };
    plugin.ok(None, None, &["daemon", "ensure"]);
    for marker in [("CLAUDECODE", "1"), ("CODEX_SANDBOX", "seatbelt")] {
        let refused = plugin.refused_with_env(Some("w1:p1"), &["human", "me", "init"], &[marker]);
        assert!(refused.contains(OVERRIDE_ARGV), "{refused}");
        assert!(refused.contains(marker.0), "{refused}");
    }
    // Nothing was recorded for the refused attempts.
    let seats: i64 = plugin
        .database()
        .query_row(
            "SELECT count(*) FROM occupant_bindings WHERE harness='human'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seats, 0);
    // The explicit override works with the marker present.
    let (code, value, stderr) = plugin.run_with_env(
        Some("w1:p1"),
        None,
        &["human", "me", "init", "--operator"],
        &[("CLAUDECODE", "1")],
    );
    assert_eq!(code, 0, "{stderr}{value}");
    assert_eq!(value["result"]["data"]["context"]["harness"], "human");
}

#[test]
fn me_init_refuses_where_stand_in_herdr_reports_claude_or_codex() {
    let (root, _scratch, socket) = scratch("htag");
    let _host = FakeHost::start(
        &socket,
        vec![
            agent_pane("w1:p1", "term-a", "claude", Some("s-1")),
            agent_pane("w1:p2", "term-b", "codex", Some("s-2")),
            pane("w1:p3", "term-c"),
        ],
    );
    let plugin = Plugin {
        state: root.join("state"),
        host: socket.clone(),
    };
    plugin.ok(None, None, &["daemon", "ensure"]);
    for (pane, kind) in [("w1:p1", "claude"), ("w1:p2", "codex")] {
        let refused = plugin.refused(Some(pane), &["human", "me", "init"]);
        assert!(refused.contains(kind), "{refused}");
        assert!(refused.contains(OVERRIDE_ARGV), "{refused}");
    }
    // A plain shell pane is fine.
    assert_eq!(
        plugin.ok(Some("w1:p3"), None, &["human", "me", "init"])["kind"],
        "checked_in"
    );
}

/// An agent holds the seat of the person's pane. `me init` is refused with the
/// override argv; `me init --operator` records the person (`operator_human`)
/// and no receipt carries the operator label. Returns the plugin for audit checks.
fn refused_then_overridden(root: &Path, socket: &Path) -> (Plugin, String) {
    let plugin = Plugin {
        state: root.join("state"),
        host: socket.to_path_buf(),
    };
    plugin.ok(None, None, &["daemon", "ensure"]);
    let seat = plugin.ok(None, None, &["seat", "resolve", "--pane", "w1:p1"])["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let agent = Some((seat.as_str(), "w1:p1"));
    plugin.ok(
        None,
        agent,
        &["check-in", "--lifecycle-event", "agent-start"],
    );

    let refused = plugin.refused(Some("w1:p1"), &["human", "me", "init"]);
    assert!(refused.contains(HUMAN_OVERRIDE_ARGV), "{refused}");
    let open = |plugin: &Plugin| -> Vec<(String, String)> {
        plugin
            .database()
            .prepare("SELECT harness,observation_provenance FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL")
            .unwrap()
            .query_map([&seat], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        open(&plugin),
        vec![("claude".to_owned(), "cooperative_top_level".to_owned())]
    );

    let me = plugin.ok(Some("w1:p1"), None, &["human", "me", "init", "--operator"]);
    assert_eq!(me["data"]["context"]["harness"], "human", "{me}");
    assert_eq!(
        open(&plugin),
        vec![("human".to_owned(), "operator_human".to_owned())]
    );

    // The person works from the pane afterwards; the label stays out of receipts.
    let thread = plugin.ok(
        Some("w1:p1"),
        None,
        &["human", "thread", "create", "--topic", "after override"],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    plugin.ok(
        Some("w1:p1"),
        None,
        &["human", "send", &thread, "--body", "hello"],
    );
    let leaked: i64 = plugin
        .database()
        .query_row(
            "SELECT (SELECT count(*) FROM messages WHERE native_observation LIKE '%operator:local-user%') \
                  + (SELECT count(*) FROM receipts WHERE ack_observation LIKE '%operator:local-user%')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);
    (plugin, seat)
}

#[test]
fn me_init_over_live_agent_binding_is_refused_then_operator_override_replaces_it() {
    let (root, _scratch, socket) = scratch("htov");
    let _host = FakeHost::start(&socket, vec![pane("w1:p1", "term-a")]);
    refused_then_overridden(&root, &socket);
}

#[test]
fn me_init_over_live_agent_binding_is_refused_then_operator_override_records_audit() {
    let (root, _scratch, socket) = scratch("htau");
    let _host = FakeHost::start(&socket, vec![pane("w1:p1", "term-a")]);
    let (plugin, _seat) = refused_then_overridden(&root, &socket);
    let uid = unsafe { libc::geteuid() };
    let audits: Vec<String> = plugin
        .database()
        .prepare(
            "SELECT operator_label FROM allocation_decisions WHERE kind='operator_human_override'",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(audits, vec![format!("operator:local-user:{uid}")]);
}

#[test]
fn flagless_command_from_human_context_with_agent_marker_refuses() {
    let (root, _scratch, socket) = scratch("htfl");
    let _host = FakeHost::start(&socket, vec![pane("w1:p1", "term-a")]);
    let plugin = Plugin {
        state: root.join("state"),
        host: socket.clone(),
    };
    plugin.ok(None, None, &["daemon", "ensure"]);
    plugin.ok(Some("w1:p1"), None, &["human", "me", "init"]);
    plugin.ok(Some("w1:p1"), None, &["human", "inbox"]);
    let refused = plugin.refused_with_env(
        Some("w1:p1"),
        &["human", "thread", "create", "--topic", "x"],
        &[("CLAUDECODE", "1")],
    );
    assert!(refused.contains(OVERRIDE_ARGV), "{refused}");
    assert!(refused.contains("CLAUDECODE"), "{refused}");
}

/// The agent seat's private `context.json` files under the state directory.
fn context_files(state: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.file_name().is_some_and(|name| name == "context.json") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(state, &mut out);
    out
}

/// TRUST-POLICY A4: a refusal that names `--operator` is honored. After
/// `me init --operator` in a pane whose environment carries agent evidence,
/// the person-pane commands in that pane work with the same environment.
#[test]
fn me_init_operator_then_person_pane_commands_work() {
    let (root, _scratch, socket) = scratch("htop");
    let _host = FakeHost::start(&socket, vec![pane("w1:p1", "term-a")]);
    let plugin = Plugin {
        state: root.join("state"),
        host: socket.clone(),
    };
    plugin.ok(None, None, &["daemon", "ensure"]);
    let seat = plugin.ok(None, None, &["seat", "resolve", "--pane", "w1:p1"])["data"]
        .as_str()
        .unwrap()
        .to_owned();
    plugin.ok(
        None,
        Some((seat.as_str(), "w1:p1")),
        &["check-in", "--lifecycle-event", "agent-start"],
    );
    let env = [("CODEX_SANDBOX", "seatbelt")];
    let refused = plugin.refused_with_env(Some("w1:p1"), &["human", "me", "init"], &env);
    assert!(refused.contains("CODEX_SANDBOX"), "{refused}");
    let (code, value, stderr) = plugin.run_with_env(
        Some("w1:p1"),
        None,
        &["human", "me", "init", "--operator"],
        &env,
    );
    assert_eq!(code, 0, "{stderr}{value}");
    assert_eq!(value["result"]["data"]["context"]["harness"], "human");
    // The recorded override covers later commands in the same pane.
    let (code, value, stderr) = plugin.run_with_env(Some("w1:p1"), None, &["human", "inbox"], &env);
    assert_eq!(code, 0, "{stderr}{value}");
    let (code, value, stderr) = plugin.run_with_env(
        Some("w1:p1"),
        None,
        &["human", "thread", "create", "--topic", "operator override"],
        &env,
    );
    assert_eq!(code, 0, "{stderr}{value}");
    // The mark covers person-pane commands only; a plain `me init` still refuses.
    let refused = plugin.refused_with_env(Some("w1:p1"), &["human", "me", "init"], &env);
    assert!(refused.contains("CODEX_SANDBOX"), "{refused}");
}

/// No local context is retired before the daemon accepts the operator
/// check-in: a rejected `me init --operator` leaves the agent's context as it
/// was. The rejection is forced with a local generation ahead of the
/// service's (the check-in cannot be prepared at an older generation).
#[test]
fn rejected_operator_check_in_leaves_agent_context_intact() {
    let (root, _scratch, socket) = scratch("htrj");
    let _host = FakeHost::start(&socket, vec![pane("w1:p1", "term-a")]);
    let plugin = Plugin {
        state: root.join("state"),
        host: socket.clone(),
    };
    plugin.ok(None, None, &["daemon", "ensure"]);
    let seat = plugin.ok(None, None, &["seat", "resolve", "--pane", "w1:p1"])["data"]
        .as_str()
        .unwrap()
        .to_owned();
    plugin.ok(
        None,
        Some((seat.as_str(), "w1:p1")),
        &["check-in", "--lifecycle-event", "agent-start"],
    );
    let files = context_files(&plugin.state);
    assert_eq!(files.len(), 1, "{files:?}");
    let mut state: Value = serde_json::from_slice(&fs::read(&files[0]).unwrap()).unwrap();
    assert_eq!(state["current"]["harness"], "Claude", "{state}");
    state["current"]["binding_generation"] = json!(99);
    fs::write(&files[0], serde_json::to_vec(&state).unwrap()).unwrap();
    let before = fs::read(&files[0]).unwrap();

    let (code, value, stderr) = plugin.run_with_env(
        Some("w1:p1"),
        None,
        &["human", "me", "init", "--operator"],
        &[],
    );
    assert_ne!(code, 0, "the check-in must be rejected: {stderr}{value}");
    let after: Value = serde_json::from_slice(&fs::read(&files[0]).unwrap()).unwrap();
    assert_eq!(after["current"], state["current"], "{after}");
    assert_eq!(after["current"]["harness"], "Claude");
    assert_eq!(fs::read(&files[0]).unwrap(), before);
    assert_eq!(plugin.operator_mark_files(), 0);
}

#[test]
fn legacy_root_person_operations_refuse_before_effects() {
    let (root, _scratch, socket) = scratch("htroot");
    let _host = FakeHost::start(&socket, vec![pane("w1:p1", "term-a")]);
    let plugin = Plugin {
        state: root.join("state"),
        host: socket,
    };
    plugin.ok(None, None, &["daemon", "ensure"]);
    let refused = plugin.refused(Some("w1:p1"), &["me", "init"]);
    assert!(refused.contains("human"), "{refused}");
    let db = plugin.database();
    let humans: i64 = db
        .query_row(
            "SELECT count(*) FROM occupant_bindings WHERE harness='human'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(humans, 0);
    plugin.ok(Some("w1:p1"), None, &["human", "me", "init"]);
    let files = context_files(&plugin.state);
    assert_eq!(files.len(), 1);
    let before = fs::read(&files[0]).unwrap();
    for args in [
        vec!["thread", "create", "--topic", "root person refusal"],
        vec!["check-in"],
        vec![
            "seat",
            "resolve",
            "--pane",
            "w1:p1",
            "--new-seat",
            "--operator",
        ],
    ] {
        let refused = plugin.refused(Some("w1:p1"), &args);
        assert!(refused.contains("human"), "{refused}");
        assert_eq!(fs::read(&files[0]).unwrap(), before);
    }
    let threads: i64 = db
        .query_row("SELECT count(*) FROM threads", [], |r| r.get(0))
        .unwrap();
    assert_eq!(threads, 0);
    assert_eq!(plugin.operator_mark_files(), 0);
    let bindings: i64 = db
        .query_row("SELECT count(*) FROM occupant_bindings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(bindings, 1);
}

/// A person can execute the real summary command emitted by accept, using
/// the same pane and the explicitly pinned private state/socket selectors.
#[test]
fn human_accept_summary_guidance_is_executable() {
    use herdr_threads::{
        cli::actor_route::InvocationActor,
        cli::commands::parse_argv,
        daemon::paths::{InstancePaths, RuntimeContext},
        test_support::spawn::SpawnOwned,
    };
    use std::{os::unix::fs::PermissionsExt, process::Stdio};
    let (root, _scratch, socket) = scratch("htsummary");
    let _host = FakeHost::start(
        &socket,
        vec![pane("w1:p1", "term-a"), pane("w1:p2", "term-b")],
    );
    let plugin = Plugin {
        state: root.join("person's state"),
        host: socket.clone(),
    };
    let context = RuntimeContext::explicit(plugin.state.clone(), socket.clone(), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    paths.prepare_instance_dir().unwrap();
    let settings = paths.instance_dir.join("settings.json");
    fs::write(&settings, r#"{"summary":{"chunk_bytes":128}}"#).unwrap();
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
    plugin.ok(None, None, &["daemon", "ensure"]);
    let person =
        plugin.ok(Some("w1:p1"), None, &["human", "me", "init"])["data"]["context"]["seat"]
            .as_str()
            .unwrap()
            .to_owned();
    let agent = plugin.ok(None, None, &["seat", "resolve", "--pane", "w1:p2"])["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let caller = Some((agent.as_str(), "w1:p2"));
    plugin.ok(
        None,
        caller,
        &["check-in", "--lifecycle-event", "summary-start"],
    );
    let thread = plugin.ok(
        None,
        caller,
        &[
            "thread",
            "create",
            "--topic",
            "herdr-threads human summary peer",
        ],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    plugin.ok(
        None,
        caller,
        &["send", &thread, "--body", &"summary content ".repeat(30)],
    );
    plugin.ok(None, caller, &["invite", &thread, "--seat", &person]);
    let mut command = herdr_threads::test_support::spawn::command(BIN);
    command
        .args(["human", "--human", "--state-dir"])
        .arg(&plugin.state)
        .arg("--host-endpoint")
        .arg(&socket)
        .args(["accept", &thread])
        .env("HERDR_PANE_ID", "w1:p1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command.spawn_owned().unwrap().wait_with_output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{} {text}",
        String::from_utf8_lossy(&output.stderr)
    );
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix("summary available: "))
        .expect("accept summary guidance");
    let argv = shlex::split(line).unwrap();
    let parsed = parse_argv(argv.clone()).unwrap();
    assert_eq!(parsed.actor, InvocationActor::Human, "{line}");
    assert_eq!(&argv[..2], &["herdr-threads", "human"]);
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["--state-dir", plugin.state.to_str().unwrap()])
    );
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["--host-endpoint", socket.to_str().unwrap()])
    );
    let mut followup = herdr_threads::test_support::spawn::command(BIN);
    followup
        .args(&argv[1..])
        .env("HERDR_PANE_ID", "w1:p1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = followup.spawn_owned().unwrap().wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "summary followup failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("work"));
}
