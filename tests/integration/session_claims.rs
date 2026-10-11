//! Agent callers are attributed from the harness environment (ht-54r.5, spec §1, decision 15):
//! the real binary run under a synthetic harness parent (`harness_shim`) against a private Herdr
//! endpoint and a detached daemon. Each test names the mutation it kills.
use super::sweep::{FakeHost, Scratch, pane};
use herdr_threads::{
    daemon::paths::{InstancePaths, RuntimeContext},
    test_support::{harness_shim, spawn::SpawnOwned},
};
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
/// Exit status of a refused agent call (`EXIT_UNSUPPORTED`).
const REFUSED: i32 = 4;

struct World {
    root: PathBuf,
    state: PathBuf,
    socket: PathBuf,
    paths: InstancePaths,
    _host: FakeHost,
    _scratch: Scratch,
}
impl World {
    /// Two plain shell panes (w1:p1, w1:p2) and, when `daemon`, a running daemon.
    fn new(daemon: bool) -> Self {
        let root = PathBuf::from(format!(
            "{}/htsc-{}",
            herdr_threads::test_support::SHORT_TMP,
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let socket = root.join("host.sock");
        let host = FakeHost::start(
            &socket,
            vec![pane("w1:p1", "terminal-a"), pane("w1:p2", "terminal-b")],
        );
        let state = root.join("state");
        let paths = InstancePaths::resolve(
            &RuntimeContext::explicit(state.clone(), socket.clone(), None).unwrap(),
        )
        .unwrap();
        paths.prepare_instance_dir().unwrap();
        let world = Self {
            root: root.clone(),
            state,
            socket,
            paths,
            _host: host,
            _scratch: Scratch(root),
        };
        if daemon {
            let out = world.human(&["daemon", "ensure"]);
            assert!(out.status.success(), "{out:?}");
        }
        world
    }

    fn routing(&self) -> Vec<String> {
        [
            "--json",
            "--state-dir",
            self.state.to_str().unwrap(),
            "--host-endpoint",
            self.socket.to_str().unwrap(),
        ]
        .map(String::from)
        .to_vec()
    }

    fn env(&self) -> Vec<(String, String)> {
        let home = |name: &str| self.root.join(name).to_string_lossy().into_owned();
        vec![
            ("HOME".into(), home("home")),
            ("CODEX_HOME".into(), home("codex")),
            ("CLAUDE_CONFIG_DIR".into(), home("claude")),
        ]
    }

    /// `human` goes right after the executable, before the routing flags.
    fn person_argv(&self, args: &[&str]) -> Vec<String> {
        let (namespace, rest) = match args.split_first() {
            Some((&"human", rest)) => (vec!["human".to_owned()], rest),
            _ => (Vec::new(), args),
        };
        namespace
            .into_iter()
            .chain(self.routing())
            .chain(rest.iter().map(|word| (*word).to_owned()))
            .collect()
    }

    /// A person: no harness parent, no harness markers.
    fn human(&self, args: &[&str]) -> Output {
        let mut command = crate::scrubbed_command(BIN);
        command.args(self.person_argv(args));
        for (key, value) in self.env() {
            command.env(key, value);
        }
        run(command)
    }

    /// The same, in pane `pane`, as a person.
    fn human_in(&self, pane: &str, args: &[&str]) -> Output {
        let mut command = crate::scrubbed_command(BIN);
        command
            .args(self.person_argv(args))
            .env("HERDR_PANE_ID", pane);
        for (key, value) in self.env() {
            command.env(key, value);
        }
        run(command)
    }

    /// An agent: the command runs as a child of a synthetic `harness` process, in `pane`, with
    /// the harness's session variable set to `session` when one is given.
    fn agent(&self, harness: &str, session: Option<&str>, pane: &str, args: &[&str]) -> Output {
        self.agent_with(harness, session, pane, args, false)
    }

    fn agent_with(
        &self,
        harness: &str,
        session: Option<&str>,
        pane: &str,
        args: &[&str],
        managed_daemon: bool,
    ) -> Output {
        let routing = self.routing();
        let argv: Vec<&str> = routing
            .iter()
            .map(String::as_str)
            .chain(args.iter().copied())
            .collect();
        let mut env = self.env();
        env.push(("HERDR_PANE_ID".into(), pane.into()));
        if let Some(session) = session {
            let variable = match harness {
                "claude" => "CLAUDE_CODE_SESSION_ID",
                "codex" => "CODEX_SESSION_ID",
                _ => "HERMES_SESSION_ID",
            };
            env.push((variable.into(), session.into()));
        }
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let command = if managed_daemon {
            harness_shim::under_codex_managed_daemon(Path::new(BIN), &argv, &env)
        } else {
            harness_shim::under_harness(harness, Path::new(BIN), &argv, &env)
        };
        run(command)
    }

    fn data(out: &Output) -> Value {
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"]["data"].clone()
    }

    fn seat(&self, pane: &str) -> String {
        Self::data(&self.human(&["seat", "resolve", "--pane", pane]))
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// (native session, role, liveness, provenance, fingerprint pid) of the seat's open bindings.
    fn bindings(&self, seat: &str) -> Vec<(String, String, String, String, Option<i64>)> {
        let db = rusqlite::Connection::open(&self.paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(3)).unwrap();
        let mut statement = db
            .prepare(
                "SELECT native_session,role,liveness,observation_provenance,liveness_pid \
                 FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY ordinal",
            )
            .unwrap();
        statement
            .query_map([seat], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}
impl Drop for World {
    fn drop(&mut self) {
        let mut command = crate::scrubbed_command(BIN);
        command
            .args(self.routing())
            .args(["daemon", "stop"])
            .envs(self.env());
        let _ = command.output();
    }
}

/// Run an owned command to completion with its output captured.
fn run(mut command: std::process::Command) -> Output {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn_owned()
        .unwrap()
        .wait_with_output()
        .unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// Kills: a CLI that builds its claim from the pane's single saved context (every process in the
// pane acting as whoever checked in last), one that drops the environment's session or the
// harness fingerprint, and a daemon that still replaces the first binding.
#[test]
fn two_claude_sessions_in_one_pane_are_distinct_bindings() {
    let world = World::new(true);
    let seat = world.seat("w1:p1");
    let a = world.agent(
        "claude",
        Some("session-A"),
        "w1:p1",
        &["check-in", "--lifecycle-event", "ev-a"],
    );
    assert!(a.status.success(), "{a:?} {}", stderr(&a));
    // B never checks in: its first plain call enrolls it.
    let b = world.agent(
        "claude",
        Some("session-B"),
        "w1:p1",
        &["thread", "create", "--topic", "from session B"],
    );
    assert!(b.status.success(), "{b:?} {}", stderr(&b));
    let open = world.bindings(&seat);
    assert_eq!(open.len(), 2, "{open:?}");
    let (first, second) = (&open[0], &open[1]);
    assert_eq!(first.0, "session-A");
    assert_eq!(second.0, "session-B");
    for binding in &open {
        assert_eq!(binding.1, "top_level", "{binding:?}");
        assert_eq!(binding.2, "connected", "{binding:?}");
        assert!(
            binding.4.is_some_and(|pid| pid > 1),
            "the binding carries the harness process fingerprint: {binding:?}"
        );
    }
    assert_eq!(first.3, "cooperative_top_level", "a lifecycle check-in");
    assert_eq!(
        second.3, "cooperative_env_session",
        "a plain call's env session"
    );
    // A's session did not move: a second call by A is a touch of its own binding.
    let again = world.agent(
        "claude",
        Some("session-A"),
        "w1:p1",
        &["thread", "create", "--topic", "from session A"],
    );
    assert!(again.status.success(), "{again:?} {}", stderr(&again));
    assert_eq!(world.bindings(&seat).len(), 2);
    // The context journal was not the source of either identity.
    let contexts = world.paths.instance_dir.join("contexts").join(&seat);
    let saved = contexts.join("context.json");
    let saved = fs::read_to_string(&saved).unwrap_or_default();
    assert!(
        !saved.contains("\"current\":{"),
        "an agent session is never saved as the pane context: {saved}"
    );
}

// Kills: a refusal that waits for the daemon (the state dir has no daemon, so a call that got that
// far would exit 3, unavailable), one that exempts reads, and a message that does not name the
// variable the agent must have.
#[test]
fn agent_call_without_session_id_is_refused_reads_included() {
    let world = World::new(false);
    for args in [
        &["inbox"][..],
        &["thread", "list"][..],
        &["thread", "create", "--topic", "x"][..],
    ] {
        let out = world.agent("claude", None, "w1:p1", args);
        assert_eq!(out.status.code(), Some(REFUSED), "{args:?}: {out:?}");
        let text = stderr(&out);
        assert!(text.contains("CLAUDE_CODE_SESSION_ID"), "{args:?}: {text}");
        assert!(text.contains("session_claim_required"), "{args:?}: {text}");
    }
    // Codex and Hermes name their own variables.
    let out = world.agent("codex", None, "w1:p1", &["thread", "list"]);
    assert_eq!(out.status.code(), Some(REFUSED), "{out:?}");
    assert!(
        stderr(&out).contains("CODEX_SESSION_ID"),
        "{}",
        stderr(&out)
    );
    let out = world.agent("hermes", None, "w1:p1", &["thread", "list"]);
    assert_eq!(out.status.code(), Some(REFUSED), "{out:?}");
    assert!(
        stderr(&out).contains("HERMES_SESSION_ID"),
        "{}",
        stderr(&out)
    );
}

// Kills: a refusal that only says the variable is missing - the pre-upgrade agent must be told
// the fix (restart the agent / re-run setup), and must never be silently attributed to the pane.
#[test]
fn claimless_agent_call_gets_the_actionable_refusal() {
    let world = World::new(false);
    for (harness, var) in [
        ("claude", "CLAUDE_CODE_SESSION_ID"),
        ("codex", "CODEX_SESSION_ID"),
        ("hermes", "HERMES_SESSION_ID"),
    ] {
        let out = world.agent(harness, None, "w1:p1", &["inbox"]);
        assert_eq!(out.status.code(), Some(REFUSED), "{harness}: {out:?}");
        let text = stderr(&out);
        assert!(
            text.contains(&format!("did not export {var}")),
            "{harness}: {text}"
        );
        assert!(text.contains("restart the agent"), "{harness}: {text}");
        assert!(text.contains("herdr-threads setup"), "{harness}: {text}");
    }
}

// Kills: a client that treats a managed-daemon Codex ancestor as an ordinary Codex, and a refusal
// without the pane hint.
#[test]
fn managed_daemon_ancestor_is_refused_with_the_pane_hint() {
    let world = World::new(false);
    let out = world.agent_with("codex", Some("codex-session"), "w1:p1", &["inbox"], true);
    assert_eq!(out.status.code(), Some(REFUSED), "{out:?}");
    let text = stderr(&out);
    assert!(text.contains("managed_daemon_refused"), "{text}");
    assert!(text.contains("codex-daemon-env"), "{text}");
    assert!(text.contains("--no-daemon"), "{text}");
}

// Kills: the human path changing: a person with no harness parent and no markers still claims a
// seat with `me init` and acts through the pane context, byte for byte as before.
#[test]
fn human_seat_is_unaffected() {
    let world = World::new(true);
    let seat = world.seat("w1:p2");
    let init = world.human_in("w1:p2", &["human", "me", "init"]);
    assert!(init.status.success(), "{init:?} {}", stderr(&init));
    let created = world.human_in(
        "w1:p2",
        &["human", "thread", "create", "--topic", "a person's thread"],
    );
    assert!(created.status.success(), "{created:?} {}", stderr(&created));
    let open = world.bindings(&seat);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(
        open[0].3,
        herdr_threads::protocol::authority::OPERATOR_HUMAN_PROVENANCE,
        "{open:?}"
    );
}

// Kills: an agent's identity kept in the pane's context journal, where a person's shell in the
// same pane would find it and act as the agent: it is derived from the harness environment, so a
// process with no harness parent has none.
#[test]
fn a_person_shell_in_an_agent_pane_has_no_agent_identity() {
    let world = World::new(true);
    let seat = world.seat("w1:p1");
    let a = world.agent(
        "claude",
        Some("session-A"),
        "w1:p1",
        &["check-in", "--lifecycle-event", "ev-a"],
    );
    assert!(a.status.success(), "{a:?} {}", stderr(&a));
    let out = world.human_in("w1:p1", &["thread", "create", "--topic", "impostor"]);
    assert!(!out.status.success(), "{out:?}");
    assert_eq!(world.bindings(&seat).len(), 1, "no binding was enrolled");
}
