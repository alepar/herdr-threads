//! Built setup and its actual live shell consumer, without runtime qualification.
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::{Command, Stdio},
};

use herdr_threads::test_support::spawn::SpawnOwned;
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const PRELUDE: &str = r#"
set -euo pipefail
export HT_CANARY_SOURCE_ONLY=1
source "$0" --out "$1/out" --model-tier off
P=$1/probe; LOGS=$P/logs; H=$2; V=$3; INFRA=0
printf 'prerequisites: python=%s node=%s npm=%s\n' "$(command -v python3)" "$NODE_BIN" "$NPM_BIN" >&2
build_env "$P"
# build_env intentionally clears the environment; restore ownership only for
# these test children, after its config/state/PATH isolation.
ENVV+=("HT_TEST_OWNER=$HT_TEST_OWNER" "HERDR_THREADS_TEST_OWNER_PID=$HERDR_THREADS_TEST_OWNER_PID" "HT_LEAK_RUN_ID=${HT_LEAK_RUN_ID:-}")
: > "$P/checks.tsv"
confdir=$P/home/.$H
"#;

struct Case {
    root: PathBuf,
    harness: &'static str,
}

impl Case {
    fn new(harness: &'static str) -> Self {
        let root = std::env::temp_dir().join(format!("ht-setup-canary-{}", uuid::Uuid::new_v4()));
        for dir in ["probe/ht", "probe/bin", "probe/logs", "home"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        symlink(BIN, root.join("probe/ht/herdr-threads")).unwrap();
        let wrapper = root.join("probe/bin").join(harness);
        fs::write(
            &wrapper,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$(dirname \"$0\")/invocations\"\ncase \"$*\" in *--version*|*--help*|*schema*) exit 93;; esac\nexit 94\n",
        )
        .unwrap();
        fs::set_permissions(wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        Self { root, harness }
    }

    fn shell(&self, body: &str) -> String {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/harness-canary.sh");
        let mut command = Command::new("bash");
        command
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("home/.claude"))
            .env("CODEX_HOME", self.root.join("home/.codex"))
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env(
                "HT_LEAK_RUN_ID",
                std::env::var_os("HT_LEAK_RUN_ID").unwrap_or_default(),
            )
            .current_dir(&self.root)
            .args(["-c", &format!("{PRELUDE}\n{body}")])
            .arg(script)
            .arg(&self.root)
            .arg(self.harness)
            .arg(if self.harness == "claude" {
                "2.1.287"
            } else {
                "0.160.0"
            })
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        herdr_threads::test_support::spawn::tag(&mut command);
        let output = command.spawn_owned().unwrap().wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        eprintln!(
            "{} shell stdout:\n{stdout}\nstderr:\n{}",
            self.harness,
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "shell failed: {:?}", output.status);
        stdout
    }

    fn setup(&self) -> (Value, String) {
        let result =
            self.shell("run_check t0.setup; cat \"$P/checks.tsv\"; cat \"$LOGS/setup.err\" >&2");
        let bytes = fs::read(self.root.join("probe/logs/setup.out")).unwrap();
        let doc: Value = serde_json::from_slice(&bytes).unwrap();
        eprintln!(
            "actual {} setup stdout:\n{}",
            self.harness,
            String::from_utf8_lossy(&bytes)
        );
        assert_eq!(doc["setup"]["action"], "installed", "{doc}");
        let observation = &doc["setup"]["harness_version"];
        assert_eq!(observation["admission"], "contract_declared");
        assert!(observation["version"].is_null());
        assert_eq!(
            observation["binary"],
            self.root
                .join("probe/bin")
                .join(self.harness)
                .display()
                .to_string()
        );
        assert_eq!(doc["setup"]["observed"], "unknown");
        assert!(
            !self.root.join("probe/bin/invocations").exists(),
            "ordinary setup executed wrapper"
        );
        (doc, result)
    }

    fn config(&self) -> PathBuf {
        self.root.join(format!(
            "probe/home/.{}/{}",
            self.harness,
            if self.harness == "claude" {
                "settings.json"
            } else {
                "hooks.json"
            }
        ))
    }

    /// Runs the actual consumer once per check, in order, inside one sourced
    /// canary shell (the shell prelude is the expensive part; each check still
    /// gets its own `t0_py setup-assert` process). Returns each check's stdout.
    fn consumers(&self, checks: &[Check]) -> Vec<String> {
        let config = self.config();
        let config_name = config.file_name().unwrap().to_str().unwrap();
        let mut body = String::new();
        for (i, check) in checks.iter().enumerate() {
            fs::write(self.root.join(format!("candidate-{i}.json")), &check.output).unwrap();
            body.push_str(&format!("printf '@@check {i}\\n'\n"));
            match &check.config {
                Config::Keep => {}
                Config::Write(bytes) => {
                    fs::write(self.root.join(format!("config-{i}")), bytes).unwrap();
                    body.push_str(&format!(
                        "cp \"$1/config-{i}\" \"$confdir/{config_name}\"\n"
                    ));
                }
                Config::Remove => {
                    body.push_str(&format!("rm \"$confdir/{config_name}\"\n"));
                }
            }
            body.push_str(&format!(
                "t0_py setup-assert \"$H\" \"$V\" \"$1/candidate-{i}.json\" \"$confdir\"\n"
            ));
        }
        let stdout = self.shell(&body);
        let results: Vec<String> = stdout
            .split("@@check ")
            .skip(1)
            .enumerate()
            .map(|(i, part)| {
                let (index, result) = part.split_once('\n').unwrap();
                assert_eq!(index, i.to_string(), "{stdout}");
                result.to_owned()
            })
            .collect();
        assert_eq!(results.len(), checks.len(), "{stdout}");
        results
    }
}

enum Config {
    Keep,
    Write(Vec<u8>),
    Remove,
}

/// One consumer check: the candidate setup output and the owned configuration
/// in place when the consumer runs. `expect` judges the consumer's stdout.
struct Check {
    label: String,
    output: Vec<u8>,
    config: Config,
    expect: Box<dyn Fn(&str) -> bool + Send>,
}

fn check(
    label: impl Into<String>,
    output: Vec<u8>,
    config: Config,
    expect: impl Fn(&str) -> bool + Send + 'static,
) -> Check {
    Check {
        label: label.into(),
        output,
        config,
        expect: Box::new(expect),
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// Kills comparing current null metadata with the independently requested V.
#[test]
fn real_claude_setup_reaches_live_canary_consumer() {
    let case = Case::new("claude");
    let (_, result) = case.setup();
    assert!(result.starts_with("t0.setup\tpass\t"), "{result}");
}

#[test]
fn real_codex_setup_reaches_live_canary_consumer() {
    let case = Case::new("codex");
    let (_, result) = case.setup();
    assert!(result.starts_with("t0.setup\tpass\t"), "{result}");
}

// Kills accepting declaration as unconditional success or trusting JSON alone.
// One test per harness (separate processes under nextest, so they run in
// parallel); each runs all its consumer checks, in order, in one canary shell.
#[test]
fn actual_claude_setup_consumer_rejects_bad_output_and_owned_configuration() {
    consumer_rejects("claude");
}

#[test]
fn actual_codex_setup_consumer_rejects_bad_output_and_owned_configuration() {
    consumer_rejects("codex");
}

fn consumer_rejects(harness: &'static str) {
    let case = Case::new(harness);
    let (original, _) = case.setup();
    let to_vec = |doc: &Value| serde_json::to_vec(doc).unwrap();
    let mut checks = vec![];
    // Optional diagnostic metadata neither matches V nor supplies proof.
    for version in [None, Some(json!("unqualified diagnostic metadata"))] {
        let mut doc = original.clone();
        let observation = doc["setup"]["harness_version"].as_object_mut().unwrap();
        match version.clone() {
            Some(version) => {
                observation.insert("version".into(), version);
            }
            None => {
                observation.remove("version");
            }
        }
        checks.push(check(
            format!("version {version:?}"),
            to_vec(&doc),
            Config::Keep,
            |r| r.starts_with("pass\t"),
        ));
    }
    for bad in [b"not json".as_slice(), b"[]", b"{}", b"{\"setup\":[]}"] {
        checks.push(check(
            format!("output {}", String::from_utf8_lossy(bad)),
            bad.to_vec(),
            Config::Keep,
            |r| r.starts_with("fail\t"),
        ));
    }
    for action in [Value::Null, json!("removed"), json!("failed")] {
        let mut doc = original.clone();
        doc["setup"]["action"] = action.clone();
        checks.push(check(
            format!("action {action}"),
            to_vec(&doc),
            Config::Keep,
            |r| r.starts_with("fail\tsetup.action"),
        ));
    }
    for observation in [
        Value::Null,
        json!([]),
        json!("bad"),
        json!({}),
        json!({"admission":"listed","version":null}),
        json!({"admission":"contract_declared","version":42}),
    ] {
        let mut doc = original.clone();
        doc["setup"]["harness_version"] = observation.clone();
        checks.push(check(
            format!("observation {observation}"),
            to_vec(&doc),
            Config::Keep,
            |r| r.starts_with("fail\t"),
        ));
    }
    let bytes = to_vec(&original);
    let installed: Value = serde_json::from_slice(&fs::read(case.config()).unwrap()).unwrap();
    let events: &[&str] = if harness == "claude" {
        &["SessionStart", "PreToolUse"]
    } else {
        &["SessionStart", "SubagentStart", "PreToolUse"]
    };
    for event in events {
        let mut config = installed.clone();
        config["hooks"].as_object_mut().unwrap().remove(*event);
        checks.push(check(
            format!("config without {event}"),
            bytes.clone(),
            Config::Write(to_vec(&config)),
            |r| r.starts_with("fail\t") && r.contains("owned"),
        ));
    }
    if harness == "claude" {
        let mut config = installed.clone();
        for group in config["hooks"]["PreToolUse"].as_array_mut().unwrap() {
            group["matcher"] = json!("Edit");
        }
        checks.push(check(
            "PreToolUse matcher Edit",
            bytes.clone(),
            Config::Write(to_vec(&config)),
            |r| r.contains("no owned PreToolUse(Bash)"),
        ));
    }
    for bad in [
        "not json",
        "[]",
        "{}",
        "{\"hooks\":{\"SessionStart\":[{\"hooks\":[{\"command\":\"echo foreign\"}]}]}}",
    ] {
        checks.push(check(
            format!("config {bad}"),
            bytes.clone(),
            Config::Write(bad.as_bytes().to_vec()),
            |r| r.starts_with("fail\t"),
        ));
    }
    checks.push(check("config removed", bytes, Config::Remove, |r| {
        r.starts_with("fail\t")
    }));
    let results = case.consumers(&checks);
    let failures: Vec<_> = checks
        .iter()
        .zip(&results)
        .filter(|(check, result)| !(check.expect)(result))
        .map(|(check, result)| format!("{harness} {}: {result}", check.label))
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(!case.config().exists());
    assert!(!case.root.join("probe/bin/invocations").exists());
}

// Kills swallowing actual setup exit errors or unavailable ht infrastructure.
#[test]
fn live_setup_consumer_keeps_command_failure_and_infra() {
    let case = Case::new("claude");
    fs::remove_file(case.root.join("probe/bin/claude")).unwrap();
    let result =
        case.shell("run_check t0.setup; cat \"$P/checks.tsv\"; printf 'infra=%s\\n' \"$INFRA\"");
    assert!(
        result.starts_with("t0.setup\tfail\tsetup claude exited"),
        "{result}"
    );
    assert!(result.contains("infra=0"));
    fs::remove_file(case.root.join("probe/ht/herdr-threads")).unwrap();
    let result =
        case.shell("run_check t0.setup; cat \"$P/checks.tsv\"; printf 'infra=%s\\n' \"$INFRA\"");
    assert!(result.starts_with("t0.setup\tfail\t"));
    assert!(result.contains("infra=1"));
}
