//! The Herdr release matrix: herdr-threads' user-facing features against one
//! official Herdr release (the README's "Requires Herdr 0.9.1 or newer").
//! `scripts/herdr-release-smoke` runs every test in this module once per
//! release listed in `tests/herdr-releases.tsv`, with that release's
//! checksum-verified binary first on PATH and `HT_HERDR_SMOKE_VERSION` naming
//! it. Each test owns a private IsolatedHerdr root (never the shared server),
//! so the tests of one release run in parallel.
//!
//! No provider, model or credentials: the agents are stand-ins. The `codex`
//! stand-in (`tests/fixtures/herdr-matrix/codex.js`, Python in npm Codex's shape) draws Codex's
//! composer, so Herdr detects and classifies it, and runs the hooks `setup
//! codex` installed exactly as Codex runs them, so every binding, delivery and
//! wake below goes through the real hooks, daemon and Herdr. The `claude`
//! stand-in only draws Claude's idle screen (launch admission and Herdr's
//! agent start; Claude's wake is its own channel, not Herdr's).
//!
//! - `herdr_release_smoke`: plugin link and startup daemon, seats in real
//!   panes, person identity, threads (create, invite/accept/reject, join,
//!   leave, archive/reopen), 1:1 pairs, priorities, required ACK and receipts,
//!   read/body/search, delivery inspection, doctor, setup/unsetup.
//! - `herdr_release_launch_and_handoff`: `launch` (codex and claude), and every
//!   handoff target an agent can choose (automatic and explicit `--new-tab`,
//!   `--new-tab --thread`, `--pane`, `--existing`), each recipient receiving
//!   its work through its own hooks.
//! - `herdr_release_wake`: nudge threshold, wake typed by Herdr into an idle
//!   agent's composer (and no wake below the threshold), in-turn delivery,
//!   ACK from the agent, and an overdue required ACK.
use herdr_threads::test_support::isolated_herdr::IsolatedHerdr;
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const PLUGIN: &str = "herdr-threads";
/// The wake texts the daemon types into an idle composer (`notification::policy::WAKE_TEXTS`):
/// Codex gets the inbox marker, a content-free harness gets the bare wake-up line.
const WAKE_TEXTS: [&str; 2] = [
    "herdr-threads: attention pending; run herdr-threads inbox",
    "herdr-threads: wake up",
];

struct Matrix {
    host: IsolatedHerdr,
    plugin: PathBuf,
    bin: PathBuf,
    version: String,
}

impl Matrix {
    /// A private server of the release under test with the plugin linked, its
    /// startup entry's daemon reachable, and the stand-ins on its PATH.
    fn new(case: &str) -> Self {
        let version = std::env::var("HT_HERDR_SMOKE_VERSION")
            .expect("HT_HERDR_SMOKE_VERSION names the Herdr release on PATH");
        let host = IsolatedHerdr::new(case).expect("Herdr required");
        let reported = host.command("herdr").arg("--version").output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&reported.stdout).trim(),
            format!("herdr {version}"),
            "the release under test is not first on PATH"
        );
        fs::create_dir_all(host.root().join("cfg")).unwrap();
        fs::write(
            host.root().join("cfg/herdr.toml"),
            "default_shell = '/bin/sh'\n",
        )
        .unwrap();
        let plugin = plugin_checkout(host.root());
        let bin = host.root().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/herdr-matrix");
        // npm Codex's shape, `<interpreter> .../codex.js`: Herdr and the caller ancestry both
        // recognize it (the stand-in's content is Python).
        executable(
            &bin.join("codex"),
            &format!(
                "#!/bin/sh\nexec python3 -I {} \"$@\"\n",
                fixture.join("codex.js").display()
            ),
        );
        executable(
            &bin.join("claude"),
            "#!/bin/sh\n[ \"$1\" = --version ] && { echo '2.1.0 (Claude Code)'; exit 0; }\n\
             printf '\\033[2J\\033[H╭──────────────────────────╮\\n│ ✻ Welcome to Claude Code │\\n\
             ╰──────────────────────────╯\\n\\n❯ \\n  ? for shortcuts\\n'\n\
             while IFS= read -r line; do [ \"$line\" = exit ] && exit 0; done\n",
        );
        // Agents' own `herdr-threads` calls reach the plugin's executable.
        std::os::unix::fs::symlink(plugin.join("bin").join(PLUGIN), bin.join(PLUGIN)).unwrap();
        let m = Matrix {
            host,
            plugin,
            bin,
            version,
        };
        let linked = m.herdr(&["plugin", "link", m.plugin.to_str().unwrap()]);
        assert_eq!(linked["result"]["plugin"]["plugin_id"], PLUGIN, "{linked}");
        m.host.start_with_path_prefix(&m.bin);
        // Herdr's own Codex integration, as a Herdr user has it: its hooks share the stand-in's
        // hooks.json with the owned ones setup adds (and drive some releases' Codex status).
        fs::create_dir_all(m.home().join(".codex")).unwrap();
        let integration = m
            .host
            .command("herdr")
            .args(["integration", "install", "codex"])
            .env("CODEX_HOME", m.home().join(".codex"))
            .output()
            .unwrap();
        assert!(
            integration.status.success(),
            "herdr integration install codex: {}",
            String::from_utf8_lossy(&integration.stderr)
        );
        let startup = wait_for("startup entry", Duration::from_secs(30), || {
            let logs = m.herdr(&["plugin", "log", "list", "--plugin", PLUGIN, "--limit", "20"]);
            logs["result"]["logs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|l| l["event"] == "startup" && l["status"] != "running")
                .cloned()
                .ok_or_else(|| logs.to_string())
        });
        assert_eq!(startup["exit_code"], 0, "{startup}");
        wait_for("daemon host reachability", Duration::from_secs(30), || {
            let health = m.ht(&["daemon", "health"]);
            let host = &health["result"]["data"]["host"];
            if host["reachability"] == "ready"
                && !health["result"]["data"]["last_reconciliation_at"].is_null()
            {
                Ok(())
            } else {
                Err(health.to_string())
            }
        });
        m
    }

    fn home(&self) -> PathBuf {
        self.host.root().join("home")
    }

    /// A Herdr input command (send-text, send-keys): success, no output to parse.
    fn herdr_input(&self, args: &[&str]) {
        let out = self.host.command("herdr").args(args).output().unwrap();
        assert!(
            out.status.success(),
            "herdr {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn herdr(&self, args: &[&str]) -> Value {
        let out = self.host.command("herdr").args(args).output().unwrap();
        json(&out, &format!("herdr {args:?}"))
    }

    /// The plugin's executable against the plugin state Herdr gives its
    /// startup entry, with the stand-ins and the agents' config homes.
    fn cli(&self) -> Command {
        let root = self.host.root();
        let mut cmd = self.host.command(self.plugin.join("bin").join(PLUGIN));
        herdr_threads::test_support::isolation::scrub_env(&mut cmd);
        cmd.envs(self.host.env())
            .env("CLAUDE_CONFIG_DIR", self.home().join(".claude"))
            .env("CODEX_HOME", self.home().join(".codex"))
            .env(
                "PATH",
                format!("{}:{}", self.bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("HERDR_THREADS_OFFLINE", "1")
            .env("NO_COLOR", "1");
        let _ = root;
        cmd
    }

    fn globals(&self, cmd: &mut Command) {
        cmd.arg("--json")
            .arg("--state-dir")
            .arg(self.host.root().join("st/herdr/plugins").join(PLUGIN))
            .arg("--host-endpoint")
            .arg(self.host.socket_path());
    }

    /// A plain (operator) call.
    fn ht(&self, args: &[&str]) -> Value {
        let mut cmd = self.cli();
        self.globals(&mut cmd);
        cmd.args(args);
        json(&cmd.output().unwrap(), &format!("herdr-threads {args:?}"))
    }

    /// The same call, returning the raw output (for refusals).
    fn ht_out(&self, args: &[&str]) -> Output {
        let mut cmd = self.cli();
        self.globals(&mut cmd);
        cmd.args(args).output().unwrap()
    }

    /// A person's call from their own shell pane (`human` first).
    fn person(&self, pane: &str, args: &[&str]) -> Value {
        let mut cmd = self.cli();
        cmd.env("HERDR_PANE_ID", pane)
            .env("HERDR_ENV", "1")
            .arg("human");
        self.globals(&mut cmd);
        cmd.args(args);
        json(&cmd.output().unwrap(), &format!("human {args:?} in {pane}"))
    }

    fn setup(&self, harness: &str) {
        let mut cmd = self.cli();
        cmd.args(["setup", harness, "--harness-binary"])
            .arg(self.bin.join(harness));
        cmd.arg("--state-dir")
            .arg(self.host.root().join("st/herdr/plugins").join(PLUGIN))
            .arg("--host-endpoint")
            .arg(self.host.socket_path());
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "setup {harness}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn workspace(&self, label: &str) -> String {
        let home = self.home();
        let created = self.herdr(&[
            "workspace",
            "create",
            "--label",
            label,
            "--no-focus",
            "--cwd",
            home.to_str().unwrap(),
        ]);
        created["result"]["root_pane"]["pane_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn tab(&self, workspace_of: &str, label: &str) -> String {
        let home = self.home();
        let workspace = workspace_of.split(':').next().unwrap();
        let created = self.herdr(&[
            "tab",
            "create",
            "--workspace",
            workspace,
            "--label",
            label,
            "--no-focus",
            "--cwd",
            home.to_str().unwrap(),
        ]);
        created["result"]["root_pane"]["pane_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// A person seat in a fresh shell pane (`human me init`).
    fn person_seat(&self, label: &str) -> (String, String) {
        let pane = self.workspace(label);
        let me = self.person(&pane, &["me", "init"]);
        assert_eq!(me["result"]["kind"], "checked_in", "{me}");
        let seat = me["result"]["data"]["context"]["seat"]
            .as_str()
            .unwrap()
            .to_owned();
        (pane, seat)
    }

    /// `launch --kind codex` into an empty pane; the stand-in's SessionStart
    /// hook checks its seat in. Returns the seat.
    fn launch_codex(&self, pane: &str) -> String {
        let mut cmd = self.cli();
        self.globals(&mut cmd);
        cmd.args([
            "launch",
            "--kind",
            "codex",
            "--pane",
            pane,
            "--harness-binary",
        ])
        .arg(self.bin.join("codex"));
        let launched = json(&cmd.output().unwrap(), "launch codex")["launch"].clone();
        assert_eq!(launched["outcome"], "started", "{launched}");
        assert_eq!(launched["pane"], pane, "{launched}");
        let seat = launched["seat"].as_str().unwrap().to_owned();
        wait_for("codex stand-in checked in", Duration::from_secs(20), || {
            let log = self.log_of(pane);
            if log
                .iter()
                .any(|e| e["kind"] == "hook" && e["event"] == "SessionStart" && e["rc"] == 0)
            {
                Ok(())
            } else {
                Err(format!("{log:?}"))
            }
        });
        seat
    }

    /// Every stand-in Codex's event log.
    fn logs(&self) -> Vec<Vec<Value>> {
        let Ok(dir) = fs::read_dir(self.home().join(".fake-codex")) else {
            return vec![];
        };
        dir.filter_map(|e| fs::read_to_string(e.ok()?.path()).ok())
            .map(|text| {
                text.lines()
                    .filter_map(|l| serde_json::from_str(l).ok())
                    .collect()
            })
            .collect()
    }

    /// The newest stand-in log started in `pane` (Herdr's HERDR_PANE_ID).
    fn log_of(&self, pane: &str) -> Vec<Value> {
        let mut logs: Vec<Vec<Value>> = self
            .logs()
            .into_iter()
            .filter(|log| log.first().is_some_and(|e| e["pane"] == pane))
            .collect();
        logs.sort_by(|a, b| a[0]["t"].as_f64().partial_cmp(&b[0]["t"].as_f64()).unwrap());
        logs.pop().unwrap_or_default()
    }

    /// Submit one line to the stand-in in `pane` (a turn) and wait until the
    /// turn ended; returns the turn's events.
    fn turn(&self, pane: &str, line: &str) -> Vec<Value> {
        let before = self.log_of(pane).len();
        self.herdr_input(&["pane", "send-text", pane, line]);
        self.herdr_input(&["pane", "send-keys", pane, "enter"]);
        wait_for(
            &format!("turn {line:?} in {pane}"),
            Duration::from_secs(40),
            || {
                let log = self.log_of(pane);
                let events = &log[before.min(log.len())..];
                if events
                    .iter()
                    .any(|e| e["kind"] == "turn_end" && e["text"] == line)
                {
                    Ok(events.to_vec())
                } else {
                    Err(format!("{events:?}"))
                }
            },
        )
    }

    /// Submit one line without waiting for the turn.
    fn start_turn(&self, pane: &str, line: &str) {
        self.herdr_input(&["pane", "send-text", pane, line]);
        self.herdr_input(&["pane", "send-keys", pane, "enter"]);
    }

    fn status(&self, pane: &str) -> String {
        self.herdr(&["agent", "get", pane])["result"]["agent"]["agent_status"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// The stand-in in `pane` runs CMD as a tool call; returns the tool event.
    fn agent_run(&self, pane: &str, command: &str) -> Value {
        let events = self.turn(pane, &format!("run: {command}"));
        events
            .iter()
            .rev()
            .find(|e| e["kind"] == "tool")
            .cloned()
            .unwrap_or_else(|| panic!("no tool event for {command}: {events:?}"))
    }

    /// The stand-in in `pane` runs `herdr-threads ARGS` as a tool call.
    fn agent_ht(&self, pane: &str, args: &str) -> Value {
        let tool = self.agent_run(pane, &format!("herdr-threads --json {args}"));
        assert_eq!(tool["rc"], 0, "agent `{args}`: {tool}");
        serde_json::from_str(tool["stdout"].as_str().unwrap())
            .unwrap_or_else(|e| panic!("agent `{args}` output: {e}: {tool}"))
    }
}

impl Drop for Matrix {
    fn drop(&mut self) {
        // The daemon Herdr's startup entry started; the root teardown that
        // follows also stops anything naming the private root.
        let _ = self.ht_out(&["daemon", "stop"]);
    }
}

fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn json(out: &Output, what: &str) -> Value {
    assert!(
        out.status.success(),
        "{what} failed ({}): stdout={} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{what}: {e}: {}", String::from_utf8_lossy(&out.stdout)))
}

fn wait_for<T>(what: &str, timeout: Duration, mut probe: impl FnMut() -> Result<T, String>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        match probe() {
            Ok(value) => return value,
            Err(last) if Instant::now() >= deadline => panic!("{what}: timed out; last: {last}"),
            Err(_) => thread::sleep(Duration::from_millis(250)),
        }
    }
}

/// A linkable checkout: the shipped manifest and scripts plus this build's
/// executable (Herdr's link never builds).
fn plugin_checkout(root: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("plugin");
    fs::create_dir_all(dir.join("bin")).unwrap();
    fs::create_dir_all(dir.join("scripts")).unwrap();
    for file in ["herdr-plugin.toml", "scripts/view.sh", "scripts/build.sh"] {
        fs::copy(src.join(file), dir.join(file)).unwrap();
    }
    fs::copy(BIN, dir.join("bin").join(PLUGIN)).unwrap();
    dir
}

fn data(v: &Value) -> &Value {
    &v["result"]["data"]
}

fn id(v: &Value) -> String {
    data(v)
        .as_str()
        .unwrap_or_else(|| panic!("no id in {v}"))
        .to_owned()
}

#[test]
#[ignore = "run per release by scripts/herdr-release-smoke (official binary on PATH)"]
fn herdr_release_smoke() {
    let m = Matrix::new("herdr-matrix-core");
    let listed = m.herdr(&["plugin", "list", "--json"]);
    assert!(
        listed["result"]["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["plugin_id"] == PLUGIN && p["enabled"] == true),
        "{listed}"
    );

    // Seats resolve in real panes; people check in from their own shell
    // panes, and an agent (B) checks in through its own hooks.
    m.setup("codex");
    let (pa, a) = m.person_seat("core-a");
    let (pc, c) = m.person_seat("core-c");
    let pb = m.workspace("core-b");
    let b = m.launch_codex(&pb);
    let resolved = m.ht(&["seat", "resolve", "--pane", &pa]);
    assert_eq!(id(&resolved), a, "{resolved}");

    // Thread, invitation and the agent's acceptance; a required ACK the agent
    // gives by displaying it in its own text inbox; receipts.
    let thread = id(&m.person(&pa, &["thread", "create", "--topic", "matrix core"]));
    m.person(&pa, &["invite", &thread, "--seat", &b]);
    m.agent_ht(&pb, &format!("accept {thread}"));
    let body = format!("matrix-ack-{}", m.version);
    let message = id(&m.person(
        &pa,
        &["send", &thread, "--body", &body, "--require-ack", &b],
    ));
    let recipients = |state: &str| {
        wait_for(&format!("receipt {state}"), Duration::from_secs(15), || {
            let r = m.person(&pa, &["delivery", "recipients", &message]);
            let items = data(&r)["items"].as_array().unwrap().clone();
            if items.len() == 1 && items[0]["seat"] == b.as_str() && items[0]["status"] == state {
                Ok(())
            } else {
                Err(r.to_string())
            }
        })
    };
    recipients("pending");
    let pending = m.person(&pa, &["pending-receipts", "--seat", &b]);
    assert!(pending.to_string().contains(&message), "{pending}");
    let shown = m.agent_run(&pb, "herdr-threads inbox");
    assert_eq!(shown["rc"], 0, "{shown}");
    assert!(shown["stdout"].as_str().unwrap().contains(&body), "{shown}");
    recipients("acknowledged");
    let members = m.person(&pa, &["participants", &thread]);
    assert!(members.to_string().contains(&b), "{members}");
    let joined = m.person(&pa, &["thread", "list", "--joined"]);
    assert!(joined.to_string().contains(&thread), "{joined}");
    m.person(&pa, &["pending-ops"]);
    m.person(&pa, &["warnings", "--seat", &a]);
    // `wake-on` stays an alias of `nudge-threshold` (all = -1, nudge = 0).
    let all = m.agent_ht(&pb, "wake-on all");
    assert!(all.to_string().contains("-1"), "{all}");
    let back = m.agent_ht(&pb, "nudge-threshold 0");
    assert!(back.to_string().contains('0'), "{back}");

    // Priorities: every level is accepted and framed on the message.
    for p in ["0", "3", "9"] {
        let sent = m.person(
            &pa,
            &["send", &thread, "-p", p, "--body", &format!("prio-{p}")],
        );
        assert!(data(&sent).is_string(), "{sent}");
    }

    // Reading never ACKs: history, a body, search.
    let read = m.person(&pa, &["read", &thread]);
    assert!(read.to_string().contains(&body), "{read}");
    let full = m.person(&pa, &["body", &message]);
    assert!(full.to_string().contains(&body), "{full}");
    let found = m.person(&pa, &["search", &body]);
    assert!(found.to_string().contains(&message), "{found}");

    // 1:1 pair: a direct send joins both seats without an invitation.
    let direct = id(&m.person(&pa, &["send", "--seat", &c, "--body", "pair-open"]));
    let cbox = m.person(&pc, &["inbox", "--machine"]);
    assert!(cbox.to_string().contains(&direct), "{cbox}");

    // Reject, join, leave, archive, reopen.
    let other = id(&m.person(&pa, &["thread", "create", "--topic", "matrix lifecycle"]));
    let invited = m.person(&pa, &["invite", &other, "--seat", &c]);
    let invitation = data(&invited)["invitation"]
        .as_str()
        .or_else(|| data(&invited).as_str())
        .unwrap_or_else(|| panic!("no invitation id: {invited}"))
        .to_owned();
    m.person(
        &pc,
        &[
            "reject",
            &other,
            "--invitation",
            &invitation,
            "--reason",
            "matrix",
        ],
    );
    m.agent_ht(&pb, &format!("join {other}"));
    m.agent_ht(&pb, &format!("leave {other}"));
    m.person(&pa, &["archive", &other]);
    m.person(&pa, &["reopen", &other]);
    let shown = m.person(&pa, &["thread", "show", &other]);
    assert!(shown.to_string().contains(&other), "{shown}");

    // Doctor reaches this Herdr.
    let doctor = m.ht_out(&["doctor"]);
    let report = String::from_utf8_lossy(&doctor.stdout);
    assert!(
        report.contains("reachab") || report.contains("host"),
        "{report}"
    );

    // Setup-status and unsetup with this Herdr's panes in place.
    let status = m.ht(&["setup-status", "codex"]);
    assert!(status.to_string().contains("installed"), "{status}");
    let mut un = m.cli();
    un.args(["unsetup", "codex", "--state-dir"])
        .arg(m.host.root().join("st/herdr/plugins").join(PLUGIN));
    let un = un.output().unwrap();
    assert!(
        un.status.success(),
        "unsetup: {}",
        String::from_utf8_lossy(&un.stderr)
    );
    assert!(
        !fs::read_to_string(m.home().join(".codex/hooks.json"))
            .unwrap_or_default()
            .contains("hook-event"),
        "unsetup left owned hooks"
    );
    println!("herdr {}: core matrix passed", m.version);
}

#[test]
#[ignore = "run per release by scripts/herdr-release-smoke (official binary on PATH)"]
fn herdr_release_launch_and_handoff() {
    let m = Matrix::new("herdr-matrix-handoff");
    m.setup("codex");
    m.setup("claude");

    // launch: codex (checks in through its hooks) and claude, in existing panes.
    let caller_pane = m.workspace("handoff-caller");
    let caller = m.launch_codex(&caller_pane);
    let claude_pane = m.tab(&caller_pane, "launch-claude");
    let mut cmd = m.cli();
    m.globals(&mut cmd);
    cmd.args([
        "launch",
        "--kind",
        "claude",
        "--pane",
        &claude_pane,
        "--harness-binary",
    ])
    .arg(m.bin.join("claude"));
    let claude = json(&cmd.output().unwrap(), "launch claude")["launch"].clone();
    assert_eq!(claude["outcome"], "started", "{claude}");

    let home = m.home();
    let cwd = home.to_str().unwrap();
    let received = |pane: &str, needle: &str| {
        wait_for(
            &format!("handoff work {needle:?} reaches {pane}"),
            Duration::from_secs(30),
            || {
                let log = m.log_of(pane);
                if log.iter().any(|e| {
                    e["kind"] == "hook" && e["context"].as_str().is_some_and(|c| c.contains(needle))
                }) {
                    Ok(())
                } else {
                    Err(format!("{log:?}"))
                }
            },
        )
    };
    let target = |out: &Value| -> (String, String) {
        let frame = out
            .get("bootstrap")
            .or_else(|| out.get("handoff"))
            .unwrap_or(out);
        assert_eq!(frame["outcome"], "started", "{out}");
        // A new-tab bootstrap names its pane at the top; an explicit handoff under `launch`.
        (
            frame["pane"]
                .as_str()
                .or_else(|| frame["launch"]["pane"].as_str())
                .unwrap_or_else(|| panic!("{out}"))
                .to_owned(),
            frame["thread"]
                .as_str()
                .unwrap_or_else(|| panic!("{out}"))
                .to_owned(),
        )
    };

    // The caller agent chooses each target itself (a real top-level session).
    let auto = m.agent_ht(
        &caller_pane,
        &format!("handoff --new-tab ho-auto --kind codex --cwd {cwd} -- handoff-auto"),
    );
    assert_eq!(auto["bootstrap"]["state"], "completed", "{auto}");
    let (pane, thread) = target(&auto);
    received(&pane, &thread);

    let explicit = m.agent_ht(
        &caller_pane,
        &format!("handoff --new-tab ho-explicit --new-thread --kind codex --cwd {cwd} -- handoff-explicit"),
    );
    let (pane, thread) = target(&explicit);
    received(&pane, &thread);

    let named = m.agent_ht(&caller_pane, "thread create --topic matrix-handoff-thread");
    let existing_thread = id(&named);
    let into_thread = m.agent_ht(
        &caller_pane,
        &format!("handoff --new-tab ho-thread --thread {existing_thread} --kind codex --cwd {cwd} -- handoff-thread"),
    );
    let (pane, _) = target(&into_thread);
    received(&pane, &existing_thread);

    let empty = m.tab(&caller_pane, "ho-pane");
    let into_pane = m.agent_ht(
        &caller_pane,
        &format!("handoff --pane {empty} --kind codex -- handoff-pane"),
    );
    let (pane, thread) = target(&into_pane);
    assert_eq!(pane, empty, "{into_pane}");
    received(&pane, &thread);

    // --existing: durable work to a live peer, no launch.
    let peer_pane = m.tab(&caller_pane, "ho-existing");
    let peer = m.launch_codex(&peer_pane);
    let delivered = m.agent_ht(
        &caller_pane,
        &format!("handoff --existing --seat {peer} -- handoff-existing"),
    );
    assert!(delivered.to_string().contains(&peer), "{delivered}");
    m.turn(&peer_pane, "next");
    received(&peer_pane, "handoff-existing");
    let _ = caller;
    println!("herdr {}: launch and handoff matrix passed", m.version);
}

#[test]
#[ignore = "run per release by scripts/herdr-release-smoke (official binary on PATH)"]
fn herdr_release_wake() {
    let m = Matrix::new("herdr-matrix-wake");
    m.setup("codex");
    let (sender_pane, _sender) = m.person_seat("wake-sender");
    let agent_pane = m.workspace("wake-agent");
    let agent = m.launch_codex(&agent_pane);
    let wakes = || {
        m.log_of(&agent_pane)
            .iter()
            .filter(|e| e["kind"] == "prompt" && WAKE_TEXTS.iter().any(|t| e["text"] == *t))
            .count()
    };
    let saw = |needle: &str| {
        m.log_of(&agent_pane).iter().any(|e| {
            e["kind"] == "hook" && e["context"].as_str().is_some_and(|c| c.contains(needle))
        })
    };

    // T = 0 (default): a default (p1) send never wakes; it arrives on the next turn.
    let silent = id(&m.person(
        &sender_pane,
        &["send", "--seat", &agent, "--body", "wake-p1"],
    ));
    thread::sleep(Duration::from_secs(8));
    assert_eq!(wakes(), 0, "a p1 send woke a T=0 seat");
    m.turn(&agent_pane, "hello");
    assert!(
        saw("wake-p1"),
        "p1 not delivered in-turn: {:?}",
        m.log_of(&agent_pane)
    );
    let _ = silent;

    // A required ACK (p0) wakes the idle agent: Herdr types the wake marker
    // into its composer and the turn's hook delivers the item.
    let ack = id(&m.person(
        &sender_pane,
        &[
            "send",
            "--seat",
            &agent,
            "--body",
            "wake-p0-ack",
            "--require-ack",
            &agent,
            "--deadline",
            "4",
        ],
    ));
    let diagnose = || {
        let out = m.ht_out(&["diagnostics", "--seat", &agent]);
        format!(
            "diagnostics: {} {} log: {:?}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
            m.log_of(&agent_pane)
                .iter()
                .map(|e| (e["kind"].clone(), e["text"].clone(), e["event"].clone()))
                .collect::<Vec<_>>()
        )
    };
    wait_for("p0 wake", Duration::from_secs(30), || {
        if wakes() >= 1 {
            Ok(())
        } else {
            Err(diagnose())
        }
    });
    wait_for(
        "p0 delivered by the wake turn",
        Duration::from_secs(10),
        || {
            if saw("wake-p0-ack") {
                Ok(())
            } else {
                Err("not delivered".into())
            }
        },
    );
    // Unacknowledged past its deadline: overdue for the sender.
    wait_for("overdue ACK", Duration::from_secs(30), || {
        let overdue = m.person(&sender_pane, &["overdue"]);
        if overdue.to_string().contains(&ack) {
            Ok(())
        } else {
            Err(overdue.to_string())
        }
    });
    // The agent ACKs it itself.
    let acked = m.agent_ht(&agent_pane, &format!("ack {ack}"));
    assert!(acked.to_string().contains(&ack), "{acked}");
    let pending = m.person(&sender_pane, &["pending-receipts", "--seat", &agent]);
    assert!(!pending.to_string().contains(&ack), "{pending}");

    // The agent raises its threshold to 3: a p3 send now wakes it (after the
    // per-seat wake spacing), a p5 send does not.
    let set = m.agent_ht(&agent_pane, "nudge-threshold 3");
    assert!(set.to_string().contains('3'), "{set}");
    thread::sleep(Duration::from_secs(32));
    let before = wakes();
    m.person(
        &sender_pane,
        &["send", "--seat", &agent, "-p", "5", "--body", "wake-p5"],
    );
    thread::sleep(Duration::from_secs(6));
    assert_eq!(wakes(), before, "a p5 send woke a T=3 seat");
    m.person(
        &sender_pane,
        &["send", "--seat", &agent, "-p", "3", "--body", "wake-p3"],
    );
    wait_for("p3 wake at T=3", Duration::from_secs(30), || {
        if wakes() > before {
            Ok(())
        } else {
            Err(format!("{:?}", m.log_of(&agent_pane)))
        }
    });
    println!("herdr {}: wake matrix passed", m.version);
}

#[test]
#[ignore = "run per release by scripts/herdr-release-smoke (official binary on PATH)"]
fn herdr_release_busy() {
    let m = Matrix::new("herdr-matrix-busy");
    m.setup("codex");
    let (sender_pane, _sender) = m.person_seat("busy-sender");
    let agent_pane = m.workspace("busy-agent");
    let agent = m.launch_codex(&agent_pane);
    let woken = || {
        m.log_of(&agent_pane)
            .iter()
            .filter(|e| e["kind"] == "prompt" && WAKE_TEXTS.iter().any(|t| e["text"] == *t))
            .count()
    };

    // A turn with two tool calls, the first busy for 8 s: Herdr reports it working.
    let line = "run: sleep 8 ;; true";
    m.start_turn(&agent_pane, line);
    wait_for("agent working", Duration::from_secs(10), || {
        let status = m.status(&agent_pane);
        if status == "working" {
            Ok(())
        } else {
            Err(status)
        }
    });
    // A p0 (`--nudge`) sent mid-turn never nudges the busy agent: the next
    // tool boundary of the same turn delivers it.
    let sent_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    m.person(
        &sender_pane,
        &["send", "--seat", &agent, "--nudge", "--body", "busy-p0"],
    );
    wait_for("busy turn end", Duration::from_secs(30), || {
        let log = m.log_of(&agent_pane);
        if log
            .iter()
            .any(|e| e["kind"] == "turn_end" && e["text"] == line)
        {
            Ok(())
        } else {
            Err(format!("{log:?}"))
        }
    });
    let log = m.log_of(&agent_pane);
    let delivered = log.iter().any(|e| {
        e["kind"] == "hook"
            && e["event"] == "PreToolUse"
            && e["context"].as_str().is_some_and(|c| c.contains("busy-p0"))
    });
    let timeline: Vec<String> = log
        .iter()
        .map(|e| {
            format!(
                "{:+.1}s {} {} rc={} ctx={:.120} out={:.300} err={:.400}",
                e["t"].as_f64().unwrap_or(0.0) - sent_at,
                e["kind"],
                e["event"],
                e["rc"],
                e["context"].as_str().unwrap_or(""),
                e["stdout"].as_str().unwrap_or(""),
                e["stderr"].as_str().unwrap_or("")
            )
        })
        .collect();
    assert!(
        delivered,
        "p0 not delivered inside the busy turn (sent at +0.0s):\n{}",
        timeline.join("\n")
    );
    // Nor after the turn: it was delivered in-turn.
    thread::sleep(Duration::from_secs(8));
    assert_eq!(
        woken(),
        0,
        "a busy (then served) agent was nudged: {:?}",
        m.log_of(&agent_pane)
    );
    println!("herdr {}: busy matrix passed", m.version);
}
