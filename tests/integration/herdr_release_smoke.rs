//! End-to-end smoke against one official Herdr release (the README's
//! "Requires Herdr 0.9.1 or newer"). `scripts/herdr-release-smoke` runs it once
//! per release listed in `tests/herdr-releases.tsv`, with that release's
//! checksum-verified binary first on PATH and `HT_HERDR_SMOKE_VERSION` naming
//! it. Everything runs in a private IsolatedHerdr root, never the shared
//! server: plugin link, the startup entry's daemon, two seats in real panes, a
//! thread with a required ACK, and a stand-in agent (a local script, no
//! provider or credentials).
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

struct Smoke {
    host: IsolatedHerdr,
    plugin: PathBuf,
}

impl Smoke {
    fn herdr(&self, args: &[&str]) -> Value {
        let out = self.host.command("herdr").args(args).output().unwrap();
        json(&out, &format!("herdr {args:?}"))
    }

    /// The plugin's linked executable against the plugin state Herdr gives
    /// its startup entry, so the CLI reaches the daemon Herdr started.
    fn cli(&self) -> Command {
        let root = self.host.root();
        let mut cmd = self.host.command(self.plugin.join("bin").join(PLUGIN));
        herdr_threads::test_support::isolation::scrub_env(&mut cmd);
        cmd.envs(self.host.env())
            .env("CLAUDE_CONFIG_DIR", root.join("claude"))
            .env("CODEX_HOME", root.join("codex"))
            .env("HERDR_THREADS_OFFLINE", "1")
            .env("NO_COLOR", "1")
            .arg("--json")
            .arg("--state-dir")
            .arg(root.join("st/herdr/plugins").join(PLUGIN))
            .arg("--host-endpoint")
            .arg(self.host.socket_path());
        cmd
    }

    fn ok(&self, seat: Option<(&str, &str)>, args: &[&str]) -> Value {
        let mut cmd = self.cli();
        cmd.args(args);
        if let Some((seat, pane)) = seat {
            cmd.args(["--cooperative-seat", seat, "--cooperative-target", pane])
                .args(["--cooperative-harness", "codex"])
                .args(["--cooperative-role", "top-level"]);
        }
        json(&cmd.output().unwrap(), &format!("herdr-threads {args:?}"))["result"].clone()
    }

    fn workspace(&self, label: &str, path_env: Option<&str>) -> String {
        let home = self.host.root().join("home");
        let mut args = vec!["workspace", "create", "--label", label, "--no-focus"];
        args.extend(["--cwd", home.to_str().unwrap()]);
        if let Some(path) = path_env {
            args.extend(["--env", path]);
        }
        self.herdr(&args)["result"]["root_pane"]["pane_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

impl Drop for Smoke {
    fn drop(&mut self) {
        // The daemon Herdr's startup entry started; the root teardown that
        // follows also stops anything naming the private root.
        let _ = self.cli().args(["daemon", "stop"]).output();
    }
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
            Err(_) => thread::sleep(Duration::from_millis(200)),
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

#[test]
#[ignore = "run per release by scripts/herdr-release-smoke (official binary on PATH)"]
fn herdr_release_smoke() {
    let expected = std::env::var("HT_HERDR_SMOKE_VERSION")
        .expect("HT_HERDR_SMOKE_VERSION names the Herdr release on PATH");
    let host = IsolatedHerdr::new("herdr-release-smoke").expect("Herdr required");
    let version = host.command("herdr").arg("--version").output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        format!("herdr {expected}"),
        "the release under test is not first on PATH"
    );
    fs::create_dir_all(host.root().join("cfg")).unwrap();
    fs::write(
        host.root().join("cfg/herdr.toml"),
        "default_shell = '/bin/sh'\n",
    )
    .unwrap();
    let plugin = plugin_checkout(host.root());
    let smoke = Smoke { host, plugin };

    // 1. Plugin links; the server runs its startup entry (`daemon ensure`).
    let linked = smoke.herdr(&["plugin", "link", smoke.plugin.to_str().unwrap()]);
    assert_eq!(linked["result"]["plugin"]["plugin_id"], PLUGIN, "{linked}");
    assert_eq!(linked["result"]["plugin"]["enabled"], true, "{linked}");
    smoke.host.start();
    let listed = smoke.herdr(&["plugin", "list", "--json"]);
    assert!(
        listed["result"]["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["plugin_id"] == PLUGIN && p["enabled"] == true),
        "{listed}"
    );
    let startup = wait_for("startup entry", Duration::from_secs(30), || {
        let logs = smoke.herdr(&["plugin", "log", "list", "--plugin", PLUGIN, "--limit", "20"]);
        logs["result"]["logs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["event"] == "startup" && l["status"] != "running")
            .cloned()
            .ok_or_else(|| logs.to_string())
    });
    assert_eq!(startup["exit_code"], 0, "{startup}");

    // 2. The daemon reaches this Herdr (ping + snapshot reconciled).
    wait_for("daemon host reachability", Duration::from_secs(30), || {
        let health = smoke.ok(None, &["daemon", "health"]);
        let host = &health["data"]["host"];
        if host["reachability"] == "ready" && !health["data"]["last_reconciliation_at"].is_null() {
            Ok(())
        } else {
            Err(format!(
                "reachability={} limitations={}",
                host["reachability"], health["data"]["limitations"]
            ))
        }
    });

    // 3. A seat resolves for each of two real panes.
    let panes = [
        smoke.workspace("smoke-a", None),
        smoke.workspace("smoke-b", None),
    ];
    let seats: Vec<String> = panes
        .iter()
        .map(|pane| {
            let seat = smoke.ok(None, &["seat", "resolve", "--pane", pane]);
            assert_eq!(seat["kind"], "seat_resolved", "{seat}");
            seat["data"].as_str().unwrap().to_owned()
        })
        .collect();
    let a = Some((seats[0].as_str(), panes[0].as_str()));
    let b = Some((seats[1].as_str(), panes[1].as_str()));
    for who in [a, b] {
        let checked = smoke.ok(who, &["check-in", "--lifecycle-event", "release-smoke"]);
        assert_eq!(checked["kind"], "checked_in", "{checked}");
    }

    // 4. Thread, invitation, a required ACK, inbox and the ACK round trip.
    let thread = smoke.ok(a, &["thread", "create", "--topic", "release smoke"])["data"]
        .as_str()
        .unwrap()
        .to_owned();
    smoke.ok(a, &["invite", &thread, "--seat", &seats[1]]);
    smoke.ok(b, &["accept", &thread]);
    let body = format!("release-smoke-{expected}");
    let message = smoke.ok(
        a,
        &["send", &thread, "--body", &body, "--require-ack", &seats[1]],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let recipient = |state: &str| {
        let r = smoke.ok(a, &["delivery", "recipients", &message]);
        let items = r["data"]["items"].as_array().unwrap().clone();
        assert!(
            items.len() == 1 && items[0]["seat"] == seats[1] && items[0]["status"] == state,
            "expected {state}: {r}"
        );
    };
    recipient("pending");
    let inbox = smoke.ok(b, &["inbox", "--machine"]);
    let items = inbox["data"]["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["message"] == message && i["body"] == body.as_str()),
        "{inbox}"
    );
    let acked = smoke.ok(b, &["ack", &message]);
    assert_eq!(
        acked["data"]["acknowledged"][0],
        message.as_str(),
        "{acked}"
    );
    recipient("acknowledged");
    let pending = smoke.ok(a, &["pending-receipts", "--seat", &seats[1]]);
    assert_eq!(pending["data"]["items"], serde_json::json!([]), "{pending}");

    // 5. A stand-in agent starts and becomes interactive-ready.
    let bin = smoke.host.root().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let standin = bin.join("codex");
    fs::write(
        &standin,
        "#!/bin/sh\nprintf '\\033[2J\\033[H› Ask Codex to do anything\\n'\n\
         while IFS= read -r line; do [ \"$line\" = exit ] && exit 0; \
         printf '\\033[2J\\033[H• observed:%s\\n› Ask Codex to do anything\\n' \"$line\"; done\n",
    )
    .unwrap();
    fs::set_permissions(&standin, fs::Permissions::from_mode(0o700)).unwrap();
    let path = format!("PATH={}:{}", bin.display(), std::env::var("PATH").unwrap());
    let pane = smoke.workspace("smoke-agent", Some(&path));
    let started = smoke.herdr(&[
        "agent",
        "start",
        "smoke",
        "--kind",
        "codex",
        "--pane",
        &pane,
        "--timeout",
        "10000",
        "--",
    ]);
    assert_eq!(
        started["result"]["agent"]["pane_id"],
        pane.as_str(),
        "{started}"
    );
    wait_for("stand-in readiness", Duration::from_secs(15), || {
        let agent = smoke.herdr(&["agent", "get", "smoke"]);
        if agent["result"]["agent"]["interactive_ready"] == true {
            Ok(())
        } else {
            Err(agent.to_string())
        }
    });
    println!("herdr {expected}: release smoke passed");
}
