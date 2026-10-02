//! Public setup CLI through the built executable: `setup`, `unsetup` and
//! `setup-status` for the user-level Claude settings and Codex hooks.json,
//! with installed-version enforcement, readable errors and documented exit
//! statuses. Harness executables (and `herdr`, for instance detection) are
//! fake scripts on a private PATH; HOME, CLAUDE_CONFIG_DIR and CODEX_HOME
//! always point into the scratch root, so the invoking user's ~/.claude,
//! ~/.codex and Herdr are never read or written. Each test names the
//! mutation it kills.

use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Scratch {
    root: PathBuf,
    state: PathBuf,
    bin: PathBuf,
    home: PathBuf,
    claude_config: PathBuf,
    codex_home: PathBuf,
}
impl Scratch {
    fn new() -> Self {
        let root = PathBuf::from(format!(
            "/private/tmp/htsetup-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let bin = root.join("bin dir");
        let home = root.join("home");
        for dir in [&bin, &home] {
            fs::create_dir(dir).unwrap();
        }
        Self {
            state: root.join("state"),
            bin,
            home,
            claude_config: root.join("claude config"),
            codex_home: root.join("codex home"),
            root,
        }
    }

    /// A fake harness whose `--version` prints exactly `line`.
    fn harness(&self, name: &str, line: &str) {
        let path = self.bin.join(name);
        fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{line}'\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_in(&self.root, args)
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        self.run_env(cwd, Some(&self.codex_home), args)
    }

    /// `codex_home` sets `CODEX_HOME` (default: the scratch one); `None`
    /// removes it so Codex resolves the scratch `$HOME/.codex`.
    fn run_env(&self, cwd: &Path, codex_home: Option<&Path>, args: &[&str]) -> Output {
        let mut command = self.command(cwd);
        match codex_home {
            Some(home) => command.env("CODEX_HOME", home),
            None => command.env_remove("CODEX_HOME"),
        };
        command
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(self.host())
            .args(args)
            .output()
            .unwrap()
    }

    /// The scratch environment with no Herdr instance on the command line
    /// or in the environment (detection uses the fake `herdr`, if any).
    fn command(&self, cwd: &Path) -> Command {
        let mut command = Command::new(BIN);
        command.envs([herdr_threads::daemon::lifecycle::test_owner_env()]);
        command
            .current_dir(cwd)
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .env("HOME", &self.home)
            .env("CLAUDE_CONFIG_DIR", &self.claude_config)
            .env("CODEX_HOME", &self.codex_home)
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("HERDR_SESSION")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_ENV");
        command
    }

    fn host(&self) -> PathBuf {
        self.root.join("host.sock")
    }

    fn settings(&self) -> PathBuf {
        self.claude_config.join("settings.json")
    }

    fn hooks(&self) -> PathBuf {
        self.codex_home.join("hooks.json")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}{}", text(&output.stdout), text(&output.stderr)))
        ["setup"]
        .clone()
}

const RULE: &str = "Bash(herdr-threads *)";

const ORIGINAL: &[u8] = b"{\n  \"permissions\": {\"allow\": [\"Bash(ls:*)\"]},\n  \"hooks\": {\"PreToolUse\": [{\"matcher\": \"Edit\", \"hooks\": [{\"type\": \"command\", \"command\": \"echo mine\"}]}]},\n  \"model\": \"x\"\n}\n";

/// Round trip on user settings (CLAUDE_CONFIG_DIR) with unrelated settings:
/// setup, idempotent re-setup, status, doctor, unsetup (byte-for-byte
/// restore), status, and a second unsetup. Kills: unsetup not restoring the
/// exact original bytes, re-setup failing because the CLI passes current
/// bytes (not the recorded baseline) to the library, setup-status/doctor
/// ignoring the manifest or CLAUDE_CONFIG_DIR, and non-idempotent unsetup.
#[test]
fn claude_install_status_remove_round_trip_restores_settings_byte_for_byte() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    fs::set_permissions(s.settings(), fs::Permissions::from_mode(0o640)).unwrap();

    let setup = s.run(&["setup", "claude"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let report = text(&setup.stdout);
    assert!(report.contains("action: installed\n"), "{report}");
    assert!(
        report.contains("harness_version.recipe: claude-hooks-2.1.283\n"),
        "{report}"
    );
    assert!(report.contains("observed: unknown\n"), "{report}");
    let installed: serde_json::Value =
        serde_json::from_slice(&fs::read(s.settings()).unwrap()).unwrap();
    assert_eq!(
        installed["permissions"]["allow"],
        serde_json::json!(["Bash(ls:*)", RULE])
    );
    assert!(
        report.contains(&format!("allow_rule.rule: {RULE}\n")),
        "{report}"
    );
    assert!(report.contains("allow_rule.ownership: owned\n"), "{report}");
    assert_eq!(installed["model"], "x");
    assert_eq!(
        installed["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "echo mine"
    );
    assert!(installed["hooks"]["SessionStart"].is_array(), "{installed}");
    assert_eq!(
        fs::metadata(s.settings()).unwrap().permissions().mode() & 0o777,
        0o640
    );

    let again = s.run(&["--json", "setup", "claude"]);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again.stderr));
    assert_eq!(json(&again)["action"], "already_installed");

    let status = s.run(&["--json", "setup-status", "claude"]);
    assert_eq!(status.status.code(), Some(0), "{}", text(&status.stderr));
    let status = json(&status);
    assert_eq!(status["installed"], true);
    assert_eq!(
        status["allow_rule"],
        serde_json::json!({"rule": RULE, "ownership": "owned", "present": true})
    );
    assert_eq!(status["current_command_matches"], true);
    assert_eq!(status["harness_version"]["supported"], true);

    // doctor reports the user-level installation.
    let doctor = s.run(&["doctor"]);
    let report = text(&doctor.stdout);
    assert!(report.contains("hooks.claude.installed: yes\n"), "{report}");
    assert!(
        report.contains(&format!("hooks.claude.allow_rule.rule: {RULE}\n")),
        "{report}"
    );
    assert!(
        report.contains(&format!(
            "hooks.claude.settings: {}\n",
            s.settings().display()
        )),
        "{report}"
    );

    let unsetup = s.run(&["unsetup", "claude"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    assert!(text(&unsetup.stdout).contains("action: removed\n"));
    assert!(text(&unsetup.stdout).contains("allow_rule: removed\n"));
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);

    let status = s.run(&["--json", "setup-status", "claude"]);
    assert_eq!(status.status.code(), Some(0));
    assert_eq!(json(&status)["installed"], false);

    let twice = s.run(&["--json", "unsetup", "claude"]);
    assert_eq!(twice.status.code(), Some(0), "{}", text(&twice.stderr));
    assert_eq!(json(&twice)["action"], "not_installed");
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);
}

/// The installed command is the hook entrypoint argv
/// `<canonical exe> --state-dir <state> --host-endpoint <socket> hook claude`,
/// shell-quoted (paths with spaces) plus the ownership comment, for both
/// declared events. Kills: omitting `--state-dir` (pane agents do not inherit
/// it) or the Herdr instance (the user-level hook must stay silent in other
/// sessions), a relative or non-canonical executable, or a subcommand other
/// than `hook claude`.
#[test]
fn installed_command_is_the_hook_entrypoint_argv() {
    let s = Scratch::new();
    s.harness("claude", "2.1.283 (Claude Code)");
    let setup = s.run(&["--json", "setup", "claude"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let exe = Path::new(BIN).canonicalize().unwrap();
    let state = s.state.display().to_string();
    let host = s.host().display().to_string();
    let expected_argv = [
        exe.to_str().unwrap(),
        "--state-dir",
        &state,
        "--host-endpoint",
        &host,
        "hook",
        "claude",
    ];
    assert_eq!(json(&setup)["hook_argv"], serde_json::json!(expected_argv));
    let quoted = expected_argv
        .iter()
        .map(|w| format!("'{}'", w.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");
    let settings: serde_json::Value =
        serde_json::from_slice(&fs::read(s.settings()).unwrap()).unwrap();
    for event in ["SessionStart", "PreToolUse"] {
        let command = settings["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(
            command.starts_with(&format!("{quoted} # herdr-threads-owner:")),
            "{event}: {command}"
        );
    }
    assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "Bash");
}

/// Setup creates an absent `settings.json` (and its config directory) as
/// `{}` and unsetup deletes exactly what setup created. Kills: refusing a
/// user without settings, and leaving a created `{}` file or directory.
#[test]
fn created_settings_are_deleted_by_unsetup() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    let setup = s.run(&["--json", "setup", "claude"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    assert_eq!(json(&setup)["created_settings"], true);
    assert!(s.settings().exists());
    let unsetup = s.run(&["--json", "unsetup", "claude"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    assert_eq!(json(&unsetup)["deleted_created_settings"], true);
    assert!(!s.claude_config.exists());
}

/// Installed-version observation is enforced: an uncovered, non-canonical or
/// missing harness is refused with exit 4 and a message naming the supported
/// recipes, and nothing is written. Kills: skipping the observation, matching
/// a version by prefix/nearest recipe, accepting a bare `X.Y.Z` line the hook
/// entrypoint would refuse, and mapping the refusal to another status.
#[test]
fn unsupported_harness_version_is_refused_with_status_four() {
    let s = Scratch::new();
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    for (line, expect) in [
        (
            "2.1.287 (Claude Code)",
            "claude 2.1.287 has no adapter recipe",
        ),
        ("2.1.284", "not a canonical"),
    ] {
        s.harness("claude", line);
        let out = s.run(&["setup", "claude"]);
        let stderr = text(&out.stderr);
        assert_eq!(out.status.code(), Some(4), "{line}: {stderr}");
        assert!(stderr.starts_with("herdr-threads: "), "{stderr}");
        assert!(stderr.contains(expect), "{line}: {stderr}");
        assert!(
            stderr.contains("claude-hooks-2.1.283 [2.1.283, 2.1.286]"),
            "{stderr}"
        );
        assert!(stderr.contains("(unsupported_harness)"), "{stderr}");
        assert!(!stderr.contains("ApiError"), "{stderr}");
        assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);
        assert!(!s.state.join("setup").exists());
    }
    fs::remove_file(s.bin.join("claude")).unwrap();
    let missing = s.run(&["setup", "claude"]);
    assert_eq!(missing.status.code(), Some(4), "{}", text(&missing.stderr));
    assert!(text(&missing.stderr).contains("no executable `claude` on PATH"));

    // Status reports the refusal without failing.
    s.harness("claude", "2.1.287 (Claude Code)");
    let status = s.run(&["--json", "setup-status", "claude"]);
    assert_eq!(status.status.code(), Some(0), "{}", text(&status.stderr));
    let status = json(&status);
    assert_eq!(status["harness_version"]["supported"], false);
    assert_eq!(status["installed"], false);
}

/// An owned group edited by hand is never removed or overwritten: unsetup and
/// re-setup exit 1 with a readable remedy and leave the file unchanged.
/// Kills: overwriting an edited owned hook, and a success status on conflict.
#[test]
fn edited_owned_hook_is_refused_with_status_one() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    let setup = s.run(&["setup", "claude"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let edited = String::from_utf8(fs::read(s.settings()).unwrap())
        .unwrap()
        .replace("\"timeout\":10", "\"timeout\":99");
    fs::write(s.settings(), &edited).unwrap();

    let unsetup = s.run(&["unsetup", "claude"]);
    let stderr = text(&unsetup.stderr);
    assert_eq!(unsetup.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("were edited or removed"), "{stderr}");
    assert!(stderr.contains("(conflict)"), "{stderr}");
    assert_eq!(fs::read_to_string(s.settings()).unwrap(), edited);

    let resetup = s.run(&["setup", "claude"]);
    assert_eq!(resetup.status.code(), Some(1), "{}", text(&resetup.stderr));
    assert!(text(&resetup.stderr).contains("unsetup claude"));
    assert_eq!(fs::read_to_string(s.settings()).unwrap(), edited);
}

/// Invalid arguments and local context exit 2 with readable text. Kills:
/// treating a relative binary as a version refusal, accepting the removed
/// `--project`, running without a (detectable) state directory, and
/// publishing into a settings file that is not a JSON object.
#[test]
fn invalid_arguments_and_settings_exit_two() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    s.harness("codex", "codex-cli 0.158.0");
    let relative = s.run(&["setup", "claude", "--harness-binary", "claude"]);
    assert_eq!(
        relative.status.code(),
        Some(2),
        "{}",
        text(&relative.stderr)
    );
    for harness in ["claude", "codex"] {
        let project = s.run(&["setup", harness, "--project", "x"]);
        assert_eq!(project.status.code(), Some(2), "{harness}");
    }
    // No --state-dir, no HERDR_PLUGIN_STATE_DIR and no `herdr` to ask.
    let no_state = s
        .command(&s.root)
        .args(["setup", "claude"])
        .output()
        .unwrap();
    assert_eq!(no_state.status.code(), Some(2));
    let stderr = text(&no_state.stderr);
    assert!(stderr.contains("state directory unknown"), "{stderr}");
    assert!(stderr.contains("no `herdr` executable"), "{stderr}");
    assert!(!s.settings().exists());
    let unknown = s.run(&["setup", "vim"]);
    assert_eq!(unknown.status.code(), Some(2));

    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), b"[1, 2]").unwrap();
    let not_object = s.run(&["setup", "claude"]);
    let stderr = text(&not_object.stderr);
    assert_eq!(not_object.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("is not a JSON object"), "{stderr}");
    assert_eq!(fs::read(s.settings()).unwrap(), b"[1, 2]");
}

/// A fake `herdr` answering the two read-only detection queries.
fn fake_herdr(s: &Scratch, plugins: &str, socket: &Path) {
    let path = s.bin.join("herdr");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n\
             'plugin list') printf '%s' '{{\"result\":{{\"plugins\":{plugins}}}}}';;\n\
             'status server') printf '%s' '{{\"running\":true,\"socket\":\"{}\"}}';;\n\
             *) exit 3;;\nesac\n",
            socket.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Outside Herdr, with no --state-dir/--host-endpoint, setup finds the
/// instance through the herdr CLI: the installed plugin's state directory
/// under XDG_STATE_HOME and the running server's socket, and records both in
/// the hook command. Kills: requiring --state-dir outside a plugin action
/// (the live-trial failure), guessing without the plugin installed, and an
/// installed hook that does not name the instance.
#[test]
fn setup_detects_the_herdr_instance_with_the_herdr_cli() {
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)");
    let xdg = s.root.join("xdg state");
    let socket = s.root.join("herdr.sock");
    fake_herdr(
        &s,
        r#"[{"plugin_id":"herdr-threads","enabled":true}]"#,
        &socket,
    );
    let out = s
        .command(&s.root)
        .env("XDG_STATE_HOME", &xdg)
        .args(["--json", "setup", "claude"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let report = json(&out);
    let state = xdg.join("herdr/plugins/herdr-threads");
    assert_eq!(report["instance"]["state_dir"], state.display().to_string());
    assert_eq!(
        report["instance"]["host_endpoint"],
        socket.display().to_string()
    );
    assert_eq!(
        report["instance"]["source"]["host_endpoint"],
        "herdr status server"
    );
    let argv: Vec<String> = serde_json::from_value(report["hook_argv"].clone()).unwrap();
    assert_eq!(
        argv[1..5],
        [
            "--state-dir".to_owned(),
            state.display().to_string(),
            "--host-endpoint".to_owned(),
            socket.display().to_string()
        ]
    );
    assert!(state.join("setup").is_dir());

    // ht-4is.8.14: once Herdr's default state directory exists it is used
    // directly (fast path), without asking `herdr plugin list` again.
    let fast = Scratch::new();
    fast.harness("claude", "2.1.285 (Claude Code)");
    fake_herdr(&fast, "[]", &socket);
    let out = fast
        .command(&fast.root)
        .env("XDG_STATE_HOME", &xdg)
        .args(["--json", "setup", "claude"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let report = json(&out);
    assert_eq!(report["instance"]["state_dir"], state.display().to_string());
    assert_eq!(
        report["instance"]["source"]["state_dir"],
        "default (XDG_STATE_HOME)"
    );

    // No default state directory and the plugin is not installed: nothing
    // is guessed or written.
    let other = Scratch::new();
    other.harness("claude", "2.1.285 (Claude Code)");
    fake_herdr(&other, "[]", &socket);
    let out = other
        .command(&other.root)
        .env("XDG_STATE_HOME", other.root.join("xdg state"))
        .args(["setup", "claude"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stdout));
    assert!(
        text(&out.stderr).contains("not installed"),
        "{}",
        text(&out.stderr)
    );
    assert!(!other.settings().exists());
}

/// Codex setup is user level: the three declared groups go into
/// `$CODEX_HOME/hooks.json` after the groups already there (Herdr's own
/// hook keeps its position and so its Codex trust key), the report names
/// each owned group's trust key, nothing outside CODEX_HOME and the state
/// directory is written, an uncovered version is refused with 4, and
/// unsetup restores the file byte for byte. Kills: writing anywhere else,
/// prepending (shifting the trust keys of existing hooks), planning without
/// the observed-version witness, and a hook command that disagrees with
/// `hook codex`.
#[test]
fn codex_setup_installs_user_hooks_and_unsetup_restores_bytes() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.158.0");
    fs::create_dir(&s.codex_home).unwrap();
    let original = br#"{"hooks":{"SessionStart":[{"hooks":[{"command":"bash '/h/herdr-agent-state.sh' session","timeout":10,"type":"command"}]}]}}"#;
    fs::write(s.hooks(), original).unwrap();
    let setup = s.run(&["--json", "setup", "codex"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let report = json(&setup);
    assert_eq!(report["scope"], "user");
    assert_eq!(report["action"], "installed");
    assert_eq!(report["harness_version"]["recipe"], "codex-hooks-v1");
    assert_eq!(
        report["events"],
        serde_json::json!(["SessionStart", "SubagentStart", "PreToolUse"])
    );
    let hooks: serde_json::Value = serde_json::from_slice(&fs::read(s.hooks()).unwrap()).unwrap();
    let start = hooks["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(start.len(), 2);
    assert_eq!(
        start[0]["hooks"][0]["command"],
        "bash '/h/herdr-agent-state.sh' session"
    );
    let command = owned_codex_command(&s);
    for event in ["SessionStart", "SubagentStart", "PreToolUse"] {
        let groups = hooks["hooks"][event].as_array().unwrap();
        let owned = groups.last().unwrap()["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(
            owned.starts_with(&format!("{command} # herdr-threads-owner:")),
            "{event}: {owned}"
        );
    }
    assert_eq!(hooks["hooks"]["PreToolUse"][0]["matcher"], "^Bash$");
    let trust = report["trust"]["hooks"].as_array().unwrap();
    let keys: Vec<&str> = trust.iter().map(|k| k["key"].as_str().unwrap()).collect();
    let file = s.hooks().display().to_string();
    assert_eq!(
        keys,
        [
            format!("{file}:session_start:1:0"),
            format!("{file}:subagent_start:0:0"),
            format!("{file}:pre_tool_use:0:0"),
        ]
    );
    assert!(trust.iter().all(|k| k["trust_recorded"] == false));
    assert!(report["trust"]["note"].as_str().unwrap().contains("/hooks"));
    // 0.158.0: the allowance is withheld, so config.toml is never created.
    assert!(!s.codex_home.join("config.toml").exists());
    assert!(!s.home.join(".codex").exists());

    let again = s.run(&["--json", "setup", "codex"]);
    assert_eq!(json(&again)["action"], "already_installed");
    let status = json(&s.run(&["--json", "setup-status", "codex"]));
    assert_eq!(status["installed"], true);
    assert_eq!(status["current_command_matches"], true);

    let text_out = s.run(&["setup", "codex"]);
    assert!(text(&text_out.stdout).contains("action: already_installed"));

    s.harness("codex", "codex-cli 0.159.2");
    let refused = s.run(&["setup", "codex"]);
    let stderr = text(&refused.stderr);
    assert_eq!(refused.status.code(), Some(4), "{stderr}");
    assert!(
        stderr.contains("codex 0.159.2 has no adapter recipe"),
        "{stderr}"
    );

    let unsetup = s.run(&["--json", "unsetup", "codex"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    assert_eq!(json(&unsetup)["action"], "removed");
    assert_eq!(fs::read(s.hooks()).unwrap(), original);
    let twice = s.run(&["--json", "unsetup", "codex"]);
    assert_eq!(json(&twice)["action"], "not_installed");
    let status = json(&s.run(&["--json", "setup-status", "codex"]));
    assert_eq!(status["installed"], false);
}

/// Snapshot of every file under `dir` (path and bytes) to prove no writes.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path.clone());
                out.push((path, Vec::new()));
            } else {
                out.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn owned_codex_command(s: &Scratch) -> String {
    let exe = Path::new(BIN).canonicalize().unwrap();
    format!(
        "'{}' '--state-dir' '{}' '--host-endpoint' '{}' 'hook' 'codex'",
        exe.display(),
        s.state.display(),
        s.host().display()
    )
}

/// B1: a user with existing Codex hooks for all three owned events, spread
/// over `$CODEX_HOME/config.toml`, `$CODEX_HOME/hooks.json` and a project
/// `.codex/config.toml`. Setup writes only the owned groups into
/// `$CODEX_HOME/hooks.json`, after the user's own groups there, names every
/// layer that already has hooks for an owned event, and leaves every other
/// file byte-identical. Kills: ignoring CODEX_HOME, not walking project
/// `.codex/` layers, dropping the `[[hooks.E]]`/dotted/`[hooks]` TOML forms or
/// the hooks.json form, copying user groups, and writing another layer.
#[test]
fn codex_setup_keeps_existing_user_hooks_in_their_own_layers() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.158.0");
    let codex_home = s.root.join("other codex home");
    fs::create_dir(&codex_home).unwrap();
    let config_toml = "model = \"m\"\n\n[[hooks.SessionStart]]\nhooks = [{ type = \"command\", command = \"hcom start\" }]\n\n[hooks]\nPreToolUse = [{ matcher = \"^Bash$\", hooks = [{ type = \"command\", command = \"hcom pre\" }] }]\n";
    fs::write(codex_home.join("config.toml"), config_toml).unwrap();
    let hooks_json = r#"{"hooks":{"SubagentStart":[{"hooks":[{"type":"command","command":"hcom child"}]}],"Stop":[{"hooks":[{"type":"command","command":"x"}]}]}}"#;
    fs::write(codex_home.join("hooks.json"), hooks_json).unwrap();
    let repo = s.root.join("repo");
    let sub = repo.join("sub");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join(".codex")).unwrap();
    fs::create_dir_all(&sub).unwrap();
    fs::write(
        repo.join(".codex").join("config.toml"),
        "hooks.SubagentStart = [{ hooks = [{ type = \"command\", command = \"proj child\" }] }]\n",
    )
    .unwrap();
    let before_repo = tree(&repo);

    let setup = s.run_env(&sub, Some(&codex_home), &["--json", "setup", "codex"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let report = json(&setup);
    let config = &report["codex_config"];
    assert_eq!(config["codex_home"], codex_home.display().to_string());
    let layers = config["layers"].as_array().unwrap();
    let events = |path: PathBuf| -> serde_json::Value {
        layers
            .iter()
            .find(|l| l["path"] == path.display().to_string())
            .unwrap_or_else(|| panic!("{} not reported: {layers:?}", path.display()))
            ["owned_events_with_hooks"]
            .clone()
    };
    assert_eq!(
        events(codex_home.join("config.toml")),
        serde_json::json!(["SessionStart", "PreToolUse"])
    );
    assert_eq!(
        events(codex_home.join("hooks.json")),
        serde_json::json!(["SubagentStart"])
    );
    assert_eq!(
        events(repo.join(".codex").join("config.toml")),
        serde_json::json!(["SubagentStart"])
    );

    // hooks.json gained exactly one owned group per event, after the user's.
    let hooks: serde_json::Value =
        serde_json::from_slice(&fs::read(codex_home.join("hooks.json")).unwrap()).unwrap();
    let child = hooks["hooks"]["SubagentStart"].as_array().unwrap();
    assert_eq!(child.len(), 2);
    assert_eq!(child[0]["hooks"][0]["command"], "hcom child");
    assert_eq!(hooks["hooks"]["Stop"][0]["hooks"][0]["command"], "x");
    let command = owned_codex_command(&s);
    for event in ["SessionStart", "SubagentStart", "PreToolUse"] {
        let groups = hooks["hooks"][event].as_array().unwrap();
        let owned: Vec<_> = groups
            .iter()
            .filter(|g| g.to_string().contains("herdr-threads-owner"))
            .collect();
        assert_eq!(owned.len(), 1, "{event}");
        assert!(
            owned[0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .starts_with(&command),
            "{event}"
        );
        assert!(!owned[0].to_string().contains("hcom"), "{event}");
    }
    assert_eq!(
        fs::read_to_string(codex_home.join("config.toml")).unwrap(),
        config_toml
    );
    assert_eq!(tree(&repo), before_repo, "project changed");
    assert!(!s.home.join(".codex").exists());
    assert!(!s.codex_home.exists());

    // Without CODEX_HOME, Codex (and setup) use $HOME/.codex.
    let default_home = s.home.join(".codex");
    fs::create_dir(&default_home).unwrap();
    fs::write(
        default_home.join("hooks.json"),
        r#"{"hooks":{"PreToolUse":[{"matcher":"^Bash$","hooks":[{"type":"command","command":"u"}]}]}}"#,
    )
    .unwrap();
    let fallback = s.run_env(&s.root, None, &["--json", "setup", "codex"]);
    assert_eq!(
        fallback.status.code(),
        Some(0),
        "{}",
        text(&fallback.stderr)
    );
    let report = json(&fallback);
    assert_eq!(
        report["codex_config"]["codex_home"],
        default_home.display().to_string()
    );
    assert_eq!(
        report["hooks_file"],
        default_home.join("hooks.json").display().to_string()
    );
}

/// Another Codex config layer that already runs the exact herdr-threads hook
/// command would run it a second time beside the owned hooks.json group:
/// setup refuses with status 1 naming the file, writes nothing, and
/// setup-status reports it. A different
/// herdr-threads command (other state dir) only warns. Kills: dropping the
/// duplicate refusal, matching only the unescaped form, and treating a
/// different installation as identical.
#[test]
fn codex_setup_refuses_a_hook_command_a_config_layer_already_runs() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.157.1");
    let codex_home = s.root.join("ch");
    fs::create_dir(&codex_home).unwrap();
    let command = owned_codex_command(&s);
    let toml = format!(
        "[[hooks.SessionStart]]\nhooks = [{{ type = \"command\", command = {} }}]\n",
        serde_json::to_string(&command).unwrap()
    );
    let config = codex_home.join("config.toml");
    fs::write(&config, &toml).unwrap();

    let refused = s.run_env(&s.root, Some(&codex_home), &["setup", "codex"]);
    let stderr = text(&refused.stderr);
    assert_eq!(refused.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains(&config.display().to_string()), "{stderr}");
    assert!(stderr.contains("a second time"), "{stderr}");
    assert!(stderr.contains("(conflict)"), "{stderr}");
    assert!(refused.stdout.is_empty());
    assert_eq!(fs::read(&config).unwrap(), toml.as_bytes());
    assert!(!codex_home.join("hooks.json").exists());

    let status = s.run_env(
        &s.root,
        Some(&codex_home),
        &["--json", "setup-status", "codex"],
    );
    assert_eq!(status.status.code(), Some(0), "{}", text(&status.stderr));
    let report = json(&status);
    assert_eq!(report["duplicate_hook"], true);
    assert_eq!(
        report["codex_config"]["layers"][0]["herdr_threads_hook"],
        "identical"
    );

    // Another installation's command (different state dir) is not identical.
    let other = command.replace(&s.state.display().to_string(), "/other/state");
    fs::write(
        &config,
        format!(
            "hooks.PreToolUse = [{{ hooks = [{{ type = \"command\", command = {} }}] }}]\n",
            serde_json::to_string(&other).unwrap()
        ),
    )
    .unwrap();
    let warned = s.run_env(&s.root, Some(&codex_home), &["--json", "setup", "codex"]);
    assert_eq!(warned.status.code(), Some(0), "{}", text(&warned.stderr));
    let report = json(&warned);
    assert_eq!(
        report["codex_config"]["layers"][0]["herdr_threads_hook"],
        "different"
    );
    assert!(
        report["warnings"][0]
            .as_str()
            .unwrap()
            .contains("different herdr-threads Codex hook"),
        "{report}"
    );
}

/// `--help` for each setup command exits 0 and documents scope and exit
/// statuses. Kills: dropping the setup help epilogue.
#[test]
fn setup_help_documents_scope_and_exit_statuses() {
    for command in ["setup", "unsetup", "setup-status"] {
        let help = Command::new(BIN)
            .envs([herdr_threads::daemon::lifecycle::test_owner_env()])
            .args([command, "--help"])
            .output()
            .unwrap();
        assert_eq!(help.status.code(), Some(0), "{command}");
        let stdout = text(&help.stdout);
        for needle in [
            "$CLAUDE_CONFIG_DIR/settings.json",
            "$CODEX_HOME/hooks.json",
            "$CODEX_HOME/config.toml",
            "herdr status server",
            "setup never writes trust",
            "sets up every detected harness",
            "Exit status:",
        ] {
            assert!(stdout.contains(needle), "{needle}: {stdout}");
        }
        assert!(!stdout.contains("--project"), "{stdout}");
    }
    let top = Command::new(BIN).arg("--help").output().unwrap();
    let stdout = text(&top.stdout);
    for command in ["setup ", "unsetup ", "setup-status "] {
        assert!(stdout.contains(command), "{stdout}");
    }
}

/// A user who already allows the identical rule keeps exactly one copy, and
/// it survives unsetup; setup-status and doctor report it as pre-existing.
/// Kills: duplicating the user's rule, recording it as owned, or removing it
/// on unsetup.
#[test]
fn preexisting_identical_allow_rule_is_not_duplicated_or_removed() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    let original = format!("{{\"permissions\": {{\"allow\": [\"{RULE}\", \"Read\"]}}}}\n");
    fs::write(s.settings(), &original).unwrap();

    let setup = s.run(&["--json", "setup", "claude"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    assert_eq!(json(&setup)["allow_rule"]["ownership"], "pre_existing");
    let installed: serde_json::Value =
        serde_json::from_slice(&fs::read(s.settings()).unwrap()).unwrap();
    assert_eq!(
        installed["permissions"]["allow"],
        serde_json::json!([RULE, "Read"])
    );

    let status = s.run(&["--json", "setup-status", "claude"]);
    let status = json(&status);
    assert_eq!(status["installed"], true);
    assert_eq!(
        status["allow_rule"],
        serde_json::json!({"rule": RULE, "ownership": "pre_existing", "present": true})
    );

    let unsetup = s.run(&["--json", "unsetup", "claude"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    assert_eq!(json(&unsetup)["allow_rule"], "left_pre_existing");
    assert_eq!(fs::read(s.settings()).unwrap(), original.as_bytes());
}

/// A recorded rule removed by hand is reported, re-setup refuses (status 1)
/// without re-adding it, and unsetup still removes the owned hooks. Kills:
/// silently re-adding the rule, reporting installed without it, or unsetup
/// refusing because the owned rule is already gone.
#[test]
fn hand_removed_allow_rule_is_reported_and_refused_but_unsetup_works() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    let setup = s.run(&["setup", "claude"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let mut v: serde_json::Value =
        serde_json::from_slice(&fs::read(s.settings()).unwrap()).unwrap();
    v["permissions"]["allow"] = serde_json::json!([]);
    fs::write(s.settings(), serde_json::to_vec(&v).unwrap()).unwrap();
    let edited = fs::read(s.settings()).unwrap();

    let status = json(&s.run(&["--json", "setup-status", "claude"]));
    assert_eq!(status["installed"], false);
    assert_eq!(status["allow_rule"]["present"], false);
    assert_eq!(status["allow_rule"]["ownership"], "owned");

    let again = s.run(&["setup", "claude"]);
    assert_eq!(again.status.code(), Some(1), "{}", text(&again.stdout));
    assert!(
        text(&again.stderr).contains(RULE),
        "{}",
        text(&again.stderr)
    );
    assert_eq!(fs::read(s.settings()).unwrap(), edited);

    let unsetup = s.run(&["unsetup", "claude"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    let after = text(&fs::read(s.settings()).unwrap());
    assert!(!after.contains("herdr-threads-owner"), "{after}");
    assert!(!after.contains(RULE), "{after}");
}

/// P6 upgrade path: an installation made by the previous setup (manifest and
/// settings holding the retired `Bash(export HERDR_THREADS_CALLER_CONTEXT=*)`
/// rule) is reported as superseded by setup-status and doctor; re-running
/// setup replaces the owned export rule with `Bash(herdr-threads *)`; unsetup
/// then restores the original bytes. Kills: reporting the old installation as
/// installed, keeping the dead export rule after re-setup, and an upgrade that
/// breaks byte-exact unsetup.
#[test]
fn resetup_replaces_the_retired_export_rule_and_unsetup_restores_bytes() {
    use sha2::Digest;
    const RETIRED: &str = "Bash(export HERDR_THREADS_CALLER_CONTEXT=*)";
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    let setup = s.run(&["--json", "setup", "claude"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let manifest_path = PathBuf::from(json(&setup)["manifest"].as_str().unwrap());

    // Rewrite into the previous setup's shape.
    let settings = text(&fs::read(s.settings()).unwrap());
    assert_eq!(settings.matches(RULE).count(), 1, "{settings}");
    let old = settings.replace(RULE, RETIRED);
    fs::write(s.settings(), &old).unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["permission"]["rule"] = serde_json::json!(RETIRED);
    manifest["permission"]["fingerprint"] = serde_json::json!(format!(
        "sha256:{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&serde_json::json!({ "allow": RETIRED })).unwrap())
    ));
    manifest["installed_fingerprint"] =
        serde_json::json!(format!("sha256:{:x}", sha2::Sha256::digest(old.as_bytes())));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let status = json(&s.run(&["--json", "setup-status", "claude"]));
    assert_eq!(status["installed"], false);
    assert_eq!(status["allow_rule"]["rule"], RULE);
    assert_eq!(status["allow_rule"]["ownership"], "superseded");
    assert_eq!(status["allow_rule"]["present"], false);
    assert!(
        status["allow_rule"]["note"]
            .as_str()
            .is_some_and(|n| n.contains(RETIRED) && n.contains("setup claude")),
        "{status}"
    );
    let doctor = s.run(&["doctor"]);
    let report = text(&doctor.stdout);
    assert!(
        report.contains("hooks.claude.allow_rule.ownership: superseded\n"),
        "{report}"
    );
    assert!(
        report.contains("hooks.claude.allow_rule.note: "),
        "{report}"
    );

    let again = s.run(&["--json", "setup", "claude"]);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again.stderr));
    assert_eq!(json(&again)["action"], "installed");
    assert_eq!(json(&again)["allow_rule"]["ownership"], "owned");
    let upgraded: serde_json::Value =
        serde_json::from_slice(&fs::read(s.settings()).unwrap()).unwrap();
    assert_eq!(
        upgraded["permissions"]["allow"],
        serde_json::json!(["Bash(ls:*)", RULE])
    );
    let status = json(&s.run(&["--json", "setup-status", "claude"]));
    assert_eq!(status["installed"], true);

    let unsetup = s.run(&["unsetup", "claude"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);
}

fn outcomes(report: &serde_json::Value) -> Vec<(String, String)> {
    report["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["harness"].as_str().unwrap().to_owned(),
                e["outcome"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(h, o)| ((*h).to_owned(), (*o).to_owned()))
        .collect()
}

/// Bare `setup` / `setup-status` / `unsetup` cover every detected harness:
/// one summary line each, the Codex trust reminder exactly once, idempotent
/// re-run, and byte-for-byte removal of both. Kills: bare setup handling only
/// one harness, repeating the trust note per harness (or not at all), and
/// bare unsetup missing a harness.
#[test]
fn bare_setup_covers_every_detected_harness_and_unsetup_removes_both() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    s.harness("codex", "codex-cli 0.158.0");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    fs::create_dir(&s.codex_home).unwrap();
    let codex_original = br#"{"hooks":{}}"#;
    fs::write(s.hooks(), codex_original).unwrap();

    let setup = s.run(&["setup"]);
    let stdout = text(&setup.stdout);
    assert_eq!(
        setup.status.code(),
        Some(0),
        "{stdout}{}",
        text(&setup.stderr)
    );
    assert!(
        stdout.contains("claude: installed: claude 2.1.284 (recipe claude-hooks-2.1.283)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("codex: installed: codex 0.158.0"),
        "{stdout}"
    );
    assert_eq!(stdout.matches("codex hook trust:").count(), 1, "{stdout}");
    assert_eq!(stdout.matches("open /hooks").count(), 1, "{stdout}");
    assert!(
        stdout
            .trim_end()
            .ends_with("Editing or moving a trusted group asks for review again")
    );

    let again = s.run(&["--json", "setup"]);
    assert_eq!(again.status.code(), Some(0));
    let report = json(&again);
    assert_eq!(report["action"], "install_all");
    assert_eq!(
        outcomes(&report),
        pairs(&[
            ("claude", "already_installed"),
            ("codex", "already_installed")
        ])
    );
    assert!(
        report["trust_reminder"]
            .as_str()
            .unwrap()
            .contains("/hooks")
    );
    assert!(report["harnesses"][1]["report"]["trust"]["note"].is_null());
    assert!(report["harnesses"][1]["report"]["trust"]["hooks"].is_array());
    assert_eq!(report["exit_status"], 0);

    let status = s.run(&["--json", "setup-status"]);
    assert_eq!(status.status.code(), Some(0));
    let status = json(&status);
    assert_eq!(
        outcomes(&status),
        pairs(&[("claude", "status"), ("codex", "status")])
    );
    for entry in status["harnesses"].as_array().unwrap() {
        assert_eq!(entry["report"]["installed"], true, "{entry}");
    }
    let status_text = text(&s.run(&["setup-status"]).stdout);
    assert!(
        status_text.starts_with("claude: installed; claude 2.1.284"),
        "{status_text}"
    );
    assert!(!status_text.contains("codex hook trust"), "{status_text}");

    let unsetup = s.run(&["unsetup"]);
    let stdout = text(&unsetup.stdout);
    assert_eq!(unsetup.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.contains("claude: removed the owned entries"),
        "{stdout}"
    );
    assert!(
        stdout.contains("codex: removed the owned entries"),
        "{stdout}"
    );
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);
    assert_eq!(fs::read(s.hooks()).unwrap(), codex_original);
    let twice = json(&s.run(&["--json", "unsetup"]));
    assert_eq!(
        outcomes(&twice),
        pairs(&[("claude", "not_installed"), ("codex", "not_installed")])
    );
}

/// A harness missing from PATH is skipped and a refused version is reported
/// with its reason; neither fails the run, and neither file is touched.
/// Kills: failing the whole run on an absent or unadmitted harness, writing
/// for a refused version, and printing the trust note with no Codex setup.
#[test]
fn bare_setup_skips_missing_and_reports_refused_harnesses() {
    let s = Scratch::new();
    s.harness("claude", "2.1.287 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    let out = s.run(&["setup"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", text(&out.stderr));
    assert!(
        stdout.contains("claude: refused: claude 2.1.287 has no adapter recipe"),
        "{stdout}"
    );
    assert!(
        stdout.contains("codex: skipped: no executable `codex` on PATH"),
        "{stdout}"
    );
    assert!(!stdout.contains("codex hook trust"), "{stdout}");
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);
    assert!(!s.codex_home.exists());

    // Only codex present: claude skipped, codex installed.
    fs::remove_file(s.bin.join("claude")).unwrap();
    s.harness("codex", "codex-cli 0.158.0");
    let report = json(&s.run(&["--json", "setup"]));
    assert_eq!(
        outcomes(&report),
        pairs(&[("claude", "skipped"), ("codex", "installed")])
    );
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);

    // Bare unsetup removes it even after codex left PATH.
    fs::remove_file(s.bin.join("codex")).unwrap();
    let removed = json(&s.run(&["--json", "unsetup"]));
    assert_eq!(
        outcomes(&removed),
        pairs(&[("claude", "not_installed"), ("codex", "removed")])
    );
    let status = json(&s.run(&["--json", "setup-status"]));
    assert_eq!(status["harnesses"][1]["detected"], false);
    assert_eq!(status["harnesses"][1]["report"]["installed"], false);

    // No harness at all: nothing to do, no Herdr instance needed.
    let none = s.command(&s.root).arg("setup").output().unwrap();
    assert_eq!(none.status.code(), Some(0), "{}", text(&none.stderr));
    assert!(text(&none.stdout).contains("claude: skipped"));
}

/// A detected harness that fails makes the run exit with its status while
/// the other harness is still set up. Bad arguments and an undetectable
/// instance exit 2 before anything is written. Kills: a zero status on a
/// failed harness, aborting the remaining harnesses, and accepting
/// --harness-binary without a harness.
#[test]
fn bare_setup_exits_nonzero_only_for_a_failed_harness() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    s.harness("codex", "codex-cli 0.158.0");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), b"[1, 2]").unwrap();
    let out = s.run(&["--json", "setup"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
    let report = json(&out);
    assert_eq!(
        outcomes(&report),
        pairs(&[("claude", "failed"), ("codex", "installed")])
    );
    assert_eq!(report["exit_status"], 2);
    assert!(
        report["harnesses"][0]["error"]
            .as_str()
            .unwrap()
            .contains("is not a JSON object")
    );
    let stderr = text(&out.stderr);
    assert!(
        stderr.starts_with("herdr-threads: setup claude: "),
        "{stderr}"
    );
    assert_eq!(fs::read(s.settings()).unwrap(), b"[1, 2]");
    assert!(s.hooks().exists());

    let binary = s.run(&["setup", "--harness-binary", "/bin/sh"]);
    assert_eq!(binary.status.code(), Some(2));
    assert!(text(&binary.stderr).contains("--harness-binary needs a harness"));

    for verb in ["setup", "unsetup"] {
        let no_state = s.command(&s.root).arg(verb).output().unwrap();
        assert_eq!(no_state.status.code(), Some(2), "{verb}");
        assert!(
            text(&no_state.stderr).contains("state directory unknown"),
            "{verb}"
        );
    }
}

/// A set-up `hooks.json` copied (with its Codex trust) into a second
/// CODEX_HOME carries the first installation's owner marker. Status and
/// doctor report it installed (adopted), setup adopts it without changing a
/// byte, re-setup is idempotent, and unsetup there removes only that file's
/// adopted groups while the first CODEX_HOME keeps its installation. Kills:
/// reporting the copy as not installed, adding a second marked set (which
/// changes Codex's trust hashes), unsetup touching the other CODEX_HOME, and
/// adopting a copy whose command really differs.
#[test]
fn copied_codex_hooks_are_adopted_without_changing_bytes() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.158.0");
    fs::create_dir(&s.codex_home).unwrap();
    let mine =
        br#"{"hooks":{"SessionStart":[{"hooks":[{"command":"echo mine","type":"command"}]}]}}"#;
    fs::write(s.hooks(), mine).unwrap();
    let setup = s.run(&["--json", "setup", "codex"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let first = fs::read(s.hooks()).unwrap();

    // The second profile: the set-up hooks.json copied byte for byte.
    let profile = s.root.join("profile codex-1");
    fs::create_dir(&profile).unwrap();
    let copied = profile.join("hooks.json");
    fs::write(&copied, &first).unwrap();
    let run = |args: &[&str]| s.run_env(&s.root, Some(&profile), args);

    let status = json(&run(&["--json", "setup-status", "codex"]));
    assert_eq!(status["installed"], true, "{status}");
    assert_eq!(status["adopted"]["recorded"], false, "{status}");
    assert_eq!(status["current_command_matches"], true, "{status}");
    let owner = status["adopted"]["owner"].as_str().unwrap().to_owned();
    assert!(
        status["command"]
            .as_str()
            .unwrap()
            .ends_with(&format!("# herdr-threads-owner:{owner}"))
    );
    let text_status = text(&run(&["setup-status"]).stdout);
    assert!(
        text_status.contains("codex: installed (adopted from another setup)"),
        "{text_status}"
    );

    let doctor = run(&["--json", "doctor"]);
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", text(&doctor.stderr)));
    let doctor = &report["doctor"];
    assert_eq!(
        doctor["hooks"]["codex"]["setup"]["installed"], true,
        "{doctor}"
    );
    assert_eq!(doctor["hooks"]["codex"]["setup"]["adopted"]["owner"], owner);
    assert!(
        doctor["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| !l.as_str().unwrap().contains("hooks are not installed")),
        "{doctor}"
    );

    let adopt = run(&["--json", "setup", "codex"]);
    assert_eq!(adopt.status.code(), Some(0), "{}", text(&adopt.stderr));
    let adopt = json(&adopt);
    assert_eq!(adopt["action"], "adopted", "{adopt}");
    assert_eq!(adopt["adopted"], owner);
    assert_eq!(fs::read(&copied).unwrap(), first);
    let again = json(&run(&["--json", "setup", "codex"]));
    assert_eq!(again["action"], "already_installed", "{again}");
    assert_eq!(fs::read(&copied).unwrap(), first);
    let status = json(&run(&["--json", "setup-status", "codex"]));
    assert_eq!(status["installed"], true);
    assert_eq!(status["adopted"]["recorded"], true);

    let unsetup = run(&["--json", "unsetup", "codex"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    let unsetup = json(&unsetup);
    assert_eq!(unsetup["action"], "removed");
    assert_eq!(unsetup["adopted"], owner);
    assert!(
        unsetup["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("review again")),
        "{unsetup}"
    );
    assert_eq!(fs::read(&copied).unwrap(), mine);
    // The first CODEX_HOME keeps its installation untouched.
    assert_eq!(fs::read(s.hooks()).unwrap(), first);
    assert_eq!(
        json(&s.run(&["--json", "setup-status", "codex"]))["installed"],
        true
    );

    // Unsetup with no prior setup in the profile also removes only the copy.
    fs::write(&copied, &first).unwrap();
    let direct = json(&run(&["--json", "unsetup", "codex"]));
    assert_eq!(direct["action"], "removed", "{direct}");
    assert_eq!(fs::read(&copied).unwrap(), mine);
    assert_eq!(fs::read(s.hooks()).unwrap(), first);

    // A copy whose command really differs (another state directory) is not
    // adopted: status stays not installed and setup installs its own set.
    let foreign = String::from_utf8(first.clone())
        .unwrap()
        .replace(&s.state.display().to_string(), "/elsewhere/state");
    fs::write(&copied, &foreign).unwrap();
    let status = json(&run(&["--json", "setup-status", "codex"]));
    assert_eq!(status["installed"], false, "{status}");
    assert!(status["adopted"].is_null());
    let installed = json(&run(&["--json", "setup", "codex"]));
    assert_eq!(installed["action"], "installed", "{installed}");
    assert!(installed["adopted"].is_null());
    let unsetup = json(&run(&["--json", "unsetup", "codex"]));
    assert_eq!(unsetup["action"], "removed");
    assert_eq!(fs::read(&copied).unwrap(), foreign.as_bytes());
    assert_eq!(fs::read(s.hooks()).unwrap(), first);
}

/// The Claude counterpart: a copied settings.json (with the allow rule) is
/// reported installed (adopted), adopted byte-identically with the rule
/// recorded as pre-existing, and unsetup removes only its hook groups.
/// Kills: Claude adoption duplicating groups or removing the rule.
#[test]
fn copied_claude_settings_are_adopted_without_changing_bytes() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    let setup = s.run(&["--json", "setup", "claude"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let first = fs::read(s.settings()).unwrap();

    let profile = s.root.join("claude profile");
    fs::create_dir(&profile).unwrap();
    let copied = profile.join("settings.json");
    fs::write(&copied, &first).unwrap();
    let run = |args: &[&str]| {
        s.command(&s.root)
            .env("CLAUDE_CONFIG_DIR", &profile)
            .arg("--state-dir")
            .arg(&s.state)
            .arg("--host-endpoint")
            .arg(s.host())
            .args(args)
            .output()
            .unwrap()
    };
    let status = json(&run(&["--json", "setup-status", "claude"]));
    assert_eq!(status["installed"], true, "{status}");
    assert_eq!(status["adopted"]["recorded"], false);
    let adopt = json(&run(&["--json", "setup", "claude"]));
    assert_eq!(adopt["action"], "adopted", "{adopt}");
    assert_eq!(adopt["allow_rule"]["ownership"], "pre_existing");
    assert_eq!(fs::read(&copied).unwrap(), first);
    let unsetup = json(&run(&["--json", "unsetup", "claude"]));
    assert_eq!(unsetup["action"], "removed", "{unsetup}");
    assert_eq!(unsetup["allow_rule"], "left_pre_existing");
    let left: serde_json::Value = serde_json::from_slice(&fs::read(&copied).unwrap()).unwrap();
    assert_eq!(
        left["permissions"]["allow"],
        serde_json::json!(["Bash(ls:*)", RULE])
    );
    assert!(
        !String::from_utf8(fs::read(&copied).unwrap())
            .unwrap()
            .contains("herdr-threads-owner")
    );
    assert_eq!(fs::read(s.settings()).unwrap(), first);
}
