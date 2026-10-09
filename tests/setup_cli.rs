//! Public setup CLI through the built executable: `setup`, `unsetup` and
//! `setup-status` for the user-level Claude settings and Codex hooks.json,
//! with executable availability, readable errors and documented exit
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

/// A spawned binary with every inherited HERDR_/CLAUDE/CODEX variable removed
/// (ht-p03.24); a test sets the variables it needs after this call.
fn scrubbed_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    herdr_threads::test_support::isolation::scrub_env(&mut command);
    command
}

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

    /// A fake `codex` for a version admitted by hook-schema match (0.159.x): it prints
    /// `codex-cli <version>` and embeds the committed hook schemas after its exit.
    fn codex_with_schemas(&self, version: &str) {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("docs/evidence/codex-158-hook-capture/schemas-0.158.0");
        let mut files: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().ends_with(".schema.json"))
            .collect();
        files.sort();
        let mut bytes =
            format!("#!/bin/sh\nprintf 'codex-cli {version}\\n'\nexit 0\n").into_bytes();
        for file in files {
            bytes.extend_from_slice(&fs::read(file).unwrap());
            bytes.push(b'\n');
        }
        let path = self.bin.join("codex");
        fs::write(&path, bytes).unwrap();
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
        let mut command = scrubbed_command(BIN);
        command
            .current_dir(cwd)
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .env("HOME", &self.home)
            .env("CLAUDE_CONFIG_DIR", &self.claude_config)
            .env("CODEX_HOME", &self.codex_home)
            // Never the host's managed policy (ht-j16.7).
            .env(
                herdr_threads::harness::claude_mod::TEST_MANAGED_SETTINGS_ENV,
                self.root.join("no-managed-settings.json"),
            )
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("HERDR_SESSION")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_ENV");
        command
    }

    /// [`Scratch::run`] with extra environment variables on the command.
    fn run_with(&self, vars: &[(&str, &std::ffi::OsStr)], args: &[&str]) -> Output {
        let mut command = self.command(&self.root);
        for (name, value) in vars {
            command.env(name, value);
        }
        command
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(self.host())
            .args(args)
            .output()
            .unwrap()
    }

    fn host(&self) -> PathBuf {
        self.root.join("host.sock")
    }

    /// The delivery mod directory `setup claude` writes.
    fn mod_dir(&self) -> PathBuf {
        self.state.join("claude-mod").join("herdr-threads")
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

#[test]
fn doctor_codex_trust_status_tracks_missing_recorded_and_unreadable_evidence() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.158.0");
    fs::create_dir(&s.codex_home).unwrap();
    let setup = s.run(&["setup", "codex"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let get = || json_doctor(&s)["hooks"]["codex"]["trust"].clone();
    let pending = get();
    assert_eq!(pending["status"], "review_required", "{pending}");
    let keys = pending["hooks"].as_array().unwrap();
    assert!(!keys.is_empty());
    let config = s.codex_home.join("config.toml");
    let mut contents = fs::read_to_string(&config).unwrap_or_default();
    for key in keys {
        contents.push_str(&format!(
            "\n[hooks.state.{}]\ntrusted_hash = \"sha256:recorded\"\n",
            serde_json::to_string(key["key"].as_str().unwrap()).unwrap()
        ));
    }
    fs::write(&config, &contents).unwrap();
    let recorded = get();
    assert_eq!(recorded["status"], "recorded_unverified", "{recorded}");
    fs::write(&config, "[broken\n").unwrap();
    let unknown = get();
    assert_eq!(unknown["status"], "unknown", "{unknown}");
}

#[test]
fn doctor_fix_refuses_unsafe_state_without_touching_harness_config() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    fs::create_dir(&s.state).unwrap();
    fs::set_permissions(&s.state, fs::Permissions::from_mode(0o777)).unwrap();
    let out = s.run(&["--json", "doctor", "fix"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let doctor = &report["doctor"];
    assert_eq!(doctor["repairs"][0]["action"], "state directory");
    assert_eq!(doctor["repairs"][0]["outcome"], "refused");
    assert!(!s.settings().exists());
}

#[test]
fn doctor_fix_installs_missing_owned_hooks_idempotently_in_scratch_home() {
    let s = Scratch::new();
    struct Stop<'a>(&'a Scratch);
    impl Drop for Stop<'_> {
        fn drop(&mut self) {
            let _ = self.0.run(&["daemon", "stop"]);
        }
    }
    let _stop = Stop(&s);
    s.harness("claude", "2.1.284 (Claude Code)");
    fs::create_dir(&s.state).unwrap();
    let first = s.run(&["--json", "doctor", "fix"]);
    let first: serde_json::Value = serde_json::from_slice(&first.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", text(&first.stderr)));
    assert_eq!(
        first["doctor"]["hooks"]["claude"]["setup"]["installed"], true,
        "{first}"
    );
    assert!(
        first["doctor"]["repairs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["action"] == "setup claude" && row["outcome"] == "attempted"),
        "{first}"
    );
    let second = s.run(&["--json", "doctor", "fix"]);
    let second: serde_json::Value = serde_json::from_slice(&second.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", text(&second.stderr)));
    assert!(
        !second["doctor"]["repairs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["action"] == "setup claude"),
        "{second}"
    );
}

#[test]
fn doctor_fix_reports_manual_codex_review_without_writing_trust() {
    let s = Scratch::new();
    struct Stop<'a>(&'a Scratch);
    impl Drop for Stop<'_> {
        fn drop(&mut self) {
            let _ = self.0.run(&["daemon", "stop"]);
        }
    }
    let _stop = Stop(&s);
    s.harness("codex", "codex-cli 0.158.0");
    fs::create_dir(&s.state).unwrap();
    fs::create_dir(&s.codex_home).unwrap();
    let setup = s.run(&["setup", "codex"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    let config = s.codex_home.join("config.toml");
    let before = fs::read(&config).ok();
    let fix = s.run(&["--json", "doctor", "fix"]);
    let fix: serde_json::Value = serde_json::from_slice(&fix.stdout).unwrap();
    assert!(
        fix["doctor"]["repairs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["action"] == "Codex hook trust" && row["outcome"] == "manual"),
        "{fix}"
    );
    assert_eq!(fs::read(&config).ok(), before);
}

#[test]
fn doctor_fix_does_not_install_codex_or_global_sandbox_allowance() {
    let s = Scratch::new();
    struct Stop<'a>(&'a Scratch);
    impl Drop for Stop<'_> {
        fn drop(&mut self) {
            let _ = self.0.run(&["daemon", "stop"]);
        }
    }
    let _stop = Stop(&s);
    s.harness("codex", "codex-cli 0.158.0");
    fs::create_dir(&s.state).unwrap();
    let fix = s.run(&["--json", "doctor", "fix"]);
    let report: serde_json::Value = serde_json::from_slice(&fix.stdout).unwrap();
    assert!(
        report["doctor"]["repairs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["action"] == "setup codex" && row["outcome"] == "manual"),
        "{report}"
    );
    assert!(!s.codex_home.join("hooks.json").exists());
    assert!(!s.codex_home.join("config.toml").exists());
}

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
    assert_eq!(status["harness_version"]["admission"], "contract_declared");

    // doctor reports the user-level installation.
    let doctor = s.run(&["doctor", "--debug"]);
    let report = text(&doctor.stdout);
    assert!(
        report.contains("hooks.claude.setup_installed: yes\n"),
        "{report}"
    );
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
            command.starts_with(&format!(
                "{quoted} '--event' '{event}' # herdr-threads-owner:"
            )),
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

/// Kills setup writing config when no executable is available.
#[test]
fn missing_harness_is_refused_without_writes() {
    let s = Scratch::new();
    let out = s.run(&["setup", "claude"]);
    assert_eq!(out.status.code(), Some(4));
    assert!(text(&out.stderr).contains("no executable `claude` on PATH"));
    assert!(!s.state.join("setup").exists());
    let status = json(&s.run(&["--json", "setup-status", "claude"]));
    assert_eq!(status["harness_version"]["admission"], "unavailable");
    assert_eq!(status["installed"], false);
}

/// Kills optional metadata selecting a different operational registration.
#[test]
fn executable_metadata_does_not_select_an_optimistic_recipe() {
    let s = Scratch::new();
    s.harness("claude", "2.1.288 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    let out = s.run(&["--json", "setup", "claude"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let report = json(&out);
    assert_eq!(
        report["harness_version"]["admission"], "contract_declared",
        "{report}"
    );
    assert_eq!(
        report["harness_version"]["version"],
        serde_json::Value::Null,
        "{report}"
    );
    // The core registration remains declared with no metadata.
    assert_eq!(
        report["harness_version"]["recipe"], "claude-hooks-2.1.283",
        "{report}"
    );
    assert_ne!(fs::read(s.settings()).unwrap(), ORIGINAL);
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
    // The delivery mod's own entry is reverted first and independently; the
    // hand-edited hook is untouched (ht-j16.7).
    let mut expected: serde_json::Value = serde_json::from_str(&edited).unwrap();
    expected.as_object_mut().unwrap().remove("env");
    assert_eq!(settings_json(&s), expected);
    let after_unsetup = fs::read_to_string(s.settings()).unwrap();

    let resetup = s.run(&["setup", "claude"]);
    assert_eq!(resetup.status.code(), Some(1), "{}", text(&resetup.stderr));
    assert!(text(&resetup.stderr).contains("unsetup claude"));
    assert_eq!(fs::read_to_string(s.settings()).unwrap(), after_unsetup);
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
/// directory is written, an older-than-supported version is refused with 4
/// (a newer unlisted one is admitted optimistically), and
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
            owned.starts_with(&format!(
                "{command} '--event' '{event}' # herdr-threads-owner:"
            )),
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

    // A newer unlisted version (fake binary, no embedded schemas) is admitted
    // optimistically; setup-status is read-only so the restore below is intact.
    s.harness("codex", "codex-cli 0.159.2");
    let optimistic = s.run(&["--json", "setup-status", "codex"]);
    assert_eq!(
        optimistic.status.code(),
        Some(0),
        "{}",
        text(&optimistic.stderr)
    );
    let optimistic = json(&optimistic);
    assert_eq!(
        optimistic["harness_version"]["admission"], "contract_declared",
        "{optimistic}"
    );
    assert_eq!(
        optimistic["harness_version"]["recipe"], "codex-hooks-v1",
        "{optimistic}"
    );

    s.harness("codex", "codex-cli 0.150.0");
    assert_eq!(s.run(&["setup", "codex"]).status.code(), Some(0));

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
        let help = scrubbed_command(BIN)
            .args([command, "--help"])
            .output()
            .unwrap();
        assert_eq!(help.status.code(), Some(0), "{command}");
        let stdout = text(&help.stdout);
        for needle in [
            "$CLAUDE_CONFIG_DIR/settings.json",
            "$CODEX_HOME/hooks.json",
            "herdr status server",
            "setup never writes trust",
            "sets up every detected harness",
            "Exit status:",
            "contract_declared",
            "Setup installs no sandbox",
        ] {
            assert!(stdout.contains(needle), "{needle}: {stdout}");
        }
        assert!(
            !stdout.contains("refuses a version no adapter recipe covers"),
            "{stdout}"
        );
        assert!(!stdout.contains("--project"), "{stdout}");
    }
    let top = scrubbed_command(BIN).arg("--help").output().unwrap();
    let stdout = text(&top.stdout);
    for command in ["setup ", "unsetup ", "setup-status "] {
        assert!(stdout.contains(command), "{stdout}");
    }
    assert!(!stdout.contains("no adapter recipe covers"), "{stdout}");
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
    let doctor = s.run(&["doctor", "--debug"]);
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
        stdout.contains("claude: installed: claude contract declared (claude-hooks-2.1.283)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("codex: installed: codex contract declared"),
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
        status_text.starts_with("claude: installed; claude contract declared"),
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

/// Kills a version gate or failing a combined setup for a missing executable.
#[test]
fn bare_setup_skips_missing_and_installs_available_harnesses() {
    let s = Scratch::new();
    s.harness("claude", "2.1.200 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    let out = s.run(&["setup"]);
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", text(&out.stderr));
    assert!(
        stdout.contains("claude: installed: claude contract declared"),
        "{stdout}"
    );
    assert!(
        stdout.contains("codex: skipped: no executable `codex` on PATH"),
        "{stdout}"
    );
    assert!(!stdout.contains("codex hook trust"), "{stdout}");
    assert_ne!(fs::read(s.settings()).unwrap(), ORIGINAL);
    assert!(!s.codex_home.exists());
    assert_eq!(s.run(&["unsetup", "claude"]).status.code(), Some(0));

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

/// Kills loss of historical allowance inspection/removal or foreign base bytes.
#[test]
fn legacy_codex_allowance_status_and_unsetup_preserve_foreign_bytes() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.3");
    fs::create_dir(&s.codex_home).unwrap();
    let config = s.codex_home.join("config.toml");
    let original = "# mine\nmodel = \"m\"\n";
    fs::write(&config, original).unwrap();
    assert_eq!(s.run(&["setup", "codex"]).status.code(), Some(0));
    seed_legacy_allowance(&s);
    let before = fs::read(&config).unwrap();
    let status = json(&s.run(&["--json", "setup-status", "codex"]));
    assert_eq!(status["sandbox"]["recorded"], true);
    assert_eq!(status["sandbox"]["present"], true);
    assert_eq!(s.run(&["setup", "codex"]).status.code(), Some(0));
    assert_eq!(fs::read(&config).unwrap(), before);
    let removed = json(&s.run(&["--json", "unsetup", "codex"]));
    assert_eq!(removed["allowance_removed"], true);
    assert_eq!(removed["hooks_removed"], true);
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    assert!(!s.hooks().exists());
}

/// Seed the historical owned-config format without asking setup to install it.
fn seed_legacy_allowance(s: &Scratch) {
    let config = s.codex_home.join("config.toml");
    let manifest = herdr_threads::cli::setup::manifest_path(&s.state, "codex-config", &config);
    herdr_threads::harness::codex_config::install(
        &config,
        &manifest,
        "/private/tmp/legacy.sock",
        &[],
    )
    .unwrap();
}

/// Kills applying the config's 1 MiB cap to a valid legacy byte-array manifest.
#[test]
fn versionless_setup_upgrades_owned_hooks_with_large_legacy_allowance_manifest() {
    use herdr_threads::harness::{codex_config, setup::InstallPhase};

    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.3");
    fs::create_dir(&s.codex_home).unwrap();
    let config = s.codex_home.join("config.toml");
    // TOML stays below 1 MiB; serializing its original bytes as a JSON array
    // produces a real historical ownership manifest between 1 and 4 MiB.
    let original = format!("# {}\nmodel = \"company\"\n", "x".repeat(350_000));
    fs::write(&config, &original).unwrap();
    assert_eq!(s.run(&["setup", "codex"]).status.code(), Some(0));
    seed_legacy_allowance(&s);
    downgrade_to_legacy(&s, "codex-user", &s.hooks());

    let manifest = herdr_threads::cli::setup::manifest_path(&s.state, "codex-config", &config);
    let config_before = fs::read(&config).unwrap();
    let manifest_before = fs::read(&manifest).unwrap();
    let hooks_before = fs::read(s.hooks()).unwrap();
    assert!(config_before.len() < 1_048_576);
    assert!(manifest_before.len() > 1_048_576);
    assert!(manifest_before.len() < 4 * 1_048_576);
    // The frozen installer's ownership precheck accepts this exact record.
    let inspection = codex_config::inspect(&config, &manifest).unwrap();
    assert!(inspection.present);
    assert_eq!(inspection.recorded.unwrap().phase, InstallPhase::Installed);

    let out = s.run(&["--json", "setup", "codex"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(fs::read(&config).unwrap(), config_before);
    assert_eq!(fs::read(&manifest).unwrap(), manifest_before);
    assert_ne!(fs::read(s.hooks()).unwrap(), hooks_before);
    let hooks: serde_json::Value = serde_json::from_slice(&fs::read(s.hooks()).unwrap()).unwrap();
    for event in ["SessionStart", "SubagentStart", "PreToolUse"] {
        let command = hooks["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(command.contains(&format!(" '--event' '{event}'")));
    }
    assert_eq!(s.run(&["unsetup", "codex"]).status.code(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
}

/// Wave 17 (ht-p03.15): other `features.network_proxy` keys the user already had stay as they
/// are, and setup, setup-status and doctor each name every one (enabling network access for
/// the sandbox makes them effective). Kills: a warning-free setup that quietly widens
/// `dangerously_allow_all_unix_sockets`, and a doctor that misses the recorded keys.
#[test]
fn preexisting_network_proxy_keys_are_warned_by_setup_status_and_doctor() {
    let s = Scratch::new();
    s.codex_with_schemas("0.159.3");
    fs::create_dir(&s.codex_home).unwrap();
    let config = s.codex_home.join("config.toml");
    fs::write(
        &config,
        "[features.network_proxy]\ndomains = [\"x\"]\ndangerously_allow_all_unix_sockets = true\n",
    )
    .unwrap();
    assert_eq!(s.run(&["setup", "codex"]).status.code(), Some(0));
    seed_legacy_allowance(&s);
    let want = |text: &str| {
        for key in ["domains", "dangerously_allow_all_unix_sockets"] {
            assert!(
                text.contains(&format!(
                    "features.network_proxy.{key} was already set in {}; enabling network \
                     access for the sandbox makes it effective",
                    config.display()
                )),
                "{key}: {text}"
            );
        }
    };
    let setup = s.run(&["--json", "setup", "codex"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    want(&json(&setup)["warnings"].to_string());
    let after = fs::read_to_string(&config).unwrap();
    assert!(after.contains("domains = [\"x\"]"), "{after}");
    assert!(
        after.contains("dangerously_allow_all_unix_sockets = true"),
        "{after}"
    );
    want(&json(&s.run(&["--json", "setup-status", "codex"]))["warnings"].to_string());
    let doctor = s.run(&["--json", "doctor"]);
    let doctor: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    want(&doctor["doctor"]["limitations"].to_string());
    want(&doctor["doctor"]["hooks"]["codex"]["sandbox_proxy_warnings"].to_string());
}

/// Kills treating a damaged legacy ownership record as absence and installing hooks.
#[test]
fn codex_install_refuses_damaged_legacy_manifest_before_hooks_change() {
    for existing in [false, true] {
        let s = Scratch::new();
        s.codex_with_schemas("0.159.3");
        fs::create_dir(&s.codex_home).unwrap();
        let hooks_before: Option<&[u8]> = existing.then_some(
            br#"{"hooks":{"SessionStart":[{"hooks":[{"command":"mine","type":"command"}]}]}}"#
                as &[u8],
        );
        if let Some(bytes) = hooks_before {
            fs::write(s.hooks(), bytes).unwrap();
        }
        let config = s.codex_home.join("config.toml");
        let config_before = "model = \"m\"\n";
        fs::write(&config, config_before).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&s.state).unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(s.state.join("setup"))
            .unwrap();
        let manifest = herdr_threads::cli::setup::manifest_path(&s.state, "codex-config", &config);
        std::os::unix::fs::symlink(s.root.join("nowhere"), &manifest).unwrap();

        let setup = s.run(&["setup", "codex"]);
        let stderr = text(&setup.stderr);
        assert_ne!(setup.status.code(), Some(0), "{stderr}");
        assert!(stderr.contains("nothing was changed"), "{stderr}");
        assert_eq!(fs::read_to_string(&config).unwrap(), config_before);
        match hooks_before {
            Some(bytes) => assert_eq!(fs::read(s.hooks()).unwrap(), bytes),
            None => assert!(!s.hooks().exists(), "created hooks.json must be removed"),
        }
        // The hooks manifest went with the rolled-back installation.
        let leftovers: Vec<_> = fs::read_dir(s.state.join("setup"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("codex-user-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}

/// Wave 25 (ht-p03.15): a state directory the fast default finds under Herdr's plugin state
/// root, whose plugin Herdr's registry does not list, is a leftover of an uninstalled plugin:
/// `setup` refuses with "plugin not installed (leftover state dir ...)" and changes nothing,
/// `doctor` warns, and `unsetup` and `setup-status` keep working on it. Kills: a setup that
/// silently takes a leftover directory, a refusal for an installed plugin, and a hard failure
/// of the other commands.
#[test]
fn setup_refuses_a_leftover_fast_state_dir_but_other_commands_work() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    let state = s.home.join(".local/state/herdr/plugins/herdr-threads");
    fs::create_dir_all(&state).unwrap();
    let registry = s.home.join(".config/herdr/plugins.json");
    fs::create_dir_all(registry.parent().unwrap()).unwrap();
    fs::write(&registry, "[]").unwrap();
    let run = |args: &[&str]| {
        s.command(&s.root)
            .arg("--host-endpoint")
            .arg(s.host())
            .args(args)
            .output()
            .unwrap()
    };

    let setup = run(&["setup", "claude"]);
    let stderr = text(&setup.stderr);
    assert_eq!(setup.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "plugin not installed (leftover state dir {})",
            state.display()
        )),
        "{stderr}"
    );
    assert!(!s.settings().exists(), "a refused setup must write nothing");
    let all = run(&["--json", "setup"]);
    assert_eq!(all.status.code(), Some(2), "{}", text(&all.stderr));

    let status = run(&["--json", "setup-status", "claude"]);
    assert_eq!(status.status.code(), Some(0), "{}", text(&status.stderr));
    let unsetup = run(&["--json", "unsetup", "claude"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    let doctor: serde_json::Value =
        serde_json::from_slice(&run(&["--json", "doctor"]).stdout).unwrap();
    assert!(
        doctor["doctor"]["limitations"]
            .to_string()
            .contains("plugin not installed (leftover state dir"),
        "{doctor}"
    );
    assert!(
        doctor["doctor"]["context"]["source"]["state_dir_leftover"].is_string(),
        "{doctor}"
    );

    // Listed by Herdr's registry: the same directory is the installed plugin's.
    fs::write(
        &registry,
        r#"[{"plugin_id":"herdr-threads","enabled":true}]"#,
    )
    .unwrap();
    let setup = run(&["--json", "setup", "claude"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    assert!(s.settings().exists());
}

/// P29 (ht-p03.15): re-running `setup` from a binary at another path than the recorded hook
/// command's is refused with a message naming the recorded path, the current path and the fix
/// (not the generic "removed by hand" causes), and the named fix works. Kills: the generic
/// conflict text for a moved executable, a message naming only one path, and a fix that does
/// not unblock setup.
#[test]
fn moved_binary_conflict_names_both_paths() {
    for harness in ["claude", "codex"] {
        let s = Scratch::new();
        if harness == "claude" {
            s.harness("claude", "2.1.284 (Claude Code)");
        } else {
            s.harness("codex", "codex-cli 0.158.0");
        }
        let first = s.run(&["--json", "setup", harness]);
        assert_eq!(first.status.code(), Some(0), "{}", text(&first.stderr));

        let moved_dir = s.root.join("moved bin");
        fs::create_dir(&moved_dir).unwrap();
        fs::copy(BIN, moved_dir.join("herdr-threads")).unwrap();
        let moved = moved_dir.join("herdr-threads").canonicalize().unwrap();
        let recorded = Path::new(BIN).canonicalize().unwrap();
        // `Scratch::command` runs the built binary: run the copy with the same environment.
        let run_moved = |verb: &str| {
            let mut command = scrubbed_command(&moved);
            let template = s.command(&s.root);
            command.current_dir(&s.root);
            for (key, value) in template.get_envs() {
                match value {
                    Some(value) => command.env(key, value),
                    None => command.env_remove(key),
                };
            }
            let out = command
                .arg("--state-dir")
                .arg(&s.state)
                .arg("--host-endpoint")
                .arg(s.host())
                .args([verb, harness])
                .output()
                .unwrap();
            (out.status.code(), text(&out.stderr))
        };
        let refused = run_moved("setup");
        assert_eq!(refused.0, Some(1), "{}", refused.1);
        assert!(
            refused.1.contains(&recorded.display().to_string())
                && refused.1.contains(&moved.display().to_string()),
            "{}",
            refused.1
        );
        assert!(
            refused
                .1
                .contains(&format!("herdr-threads unsetup {harness}"))
                && !refused.1.contains("removed by hand"),
            "{}",
            refused.1
        );
        // The named fix: unsetup (from the moved binary), then setup.
        assert_eq!(run_moved("unsetup").0, Some(0));
        assert_eq!(run_moved("setup").0, Some(0));
    }
}

/// Wave 17 docs (ht-p03.15): docs/install.md says a symlinked `settings.json` or `hooks.json`
/// is refused (status 2) and neither the link nor its target is changed; setup does not follow
/// the link. Kills: a docs statement that outlives a change in behavior, and a setup that
/// writes through the link.
#[test]
fn symlinked_dotfiles_are_refused_and_left_alone() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    s.harness("codex", "codex-cli 0.158.0");
    fs::create_dir(&s.claude_config).unwrap();
    fs::create_dir(&s.codex_home).unwrap();
    let dotfiles = s.root.join("dotfiles");
    fs::create_dir(&dotfiles).unwrap();
    for (harness, link) in [("claude", s.settings()), ("codex", s.hooks())] {
        let target = dotfiles.join(format!("{harness}.json"));
        fs::write(&target, "{}\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let out = s.run(&["setup", harness]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{harness}: {}",
            text(&out.stderr)
        );
        assert!(
            text(&out.stderr).contains("symlink"),
            "{}",
            text(&out.stderr)
        );
        assert_eq!(fs::read(&target).unwrap(), b"{}\n");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

/// The doctor JSON for `hooks.<harness>.installed` (ht-p03.47): Claude's is
/// the binary object mirroring Codex's, with no claude on PATH the stub fill
/// (`not_found`, nulls); the hooks-installed flag moved to `hooks.claude.setup`.
/// Kills: a bool left at `hooks.claude.installed`, a fifth or missing key,
/// a non-null field for an absent claude, the flag missing from `setup`, and a
/// Codex admission outside the closed set.
#[test]
fn doctor_json_claude_installed_is_the_admission_object() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.158.0");
    let report = json_doctor(&s);
    let claude = &report["hooks"]["claude"];
    let installed = claude["installed"].as_object().expect("object, not bool");
    let mut keys: Vec<_> = installed.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["admission", "binary", "recipe", "version"],
        "{claude}"
    );
    assert_eq!(installed["admission"], "not_found");
    for key in ["binary", "version", "recipe"] {
        assert!(installed[key].is_null(), "{key}: {claude}");
    }
    assert!(claude["setup"]["installed"].is_boolean(), "{claude}");
    assert_eq!(claude["setup"]["installed"], false);
    assert_eq!(
        claude["setup"]["settings"],
        s.settings().display().to_string()
    );
    let codex = &report["hooks"]["codex"]["installed"];
    let closed = [
        "contract_declared",
        "listed",
        "schema-matched, live-unverified",
        "optimistic",
        "refused",
        "not_found",
    ];
    assert!(
        closed.contains(&codex["admission"].as_str().unwrap()),
        "{codex}"
    );
    assert_eq!(codex["admission"], "contract_declared", "{codex}");
    assert!(codex["version"].is_null(), "{codex}");
    assert_eq!(codex["recipe"], "codex-hooks-v1", "{codex}");
    assert!(
        codex["binary"].as_str().unwrap().ends_with("/codex"),
        "{codex}"
    );
    assert!(codex["evidence"].is_string(), "{codex}");
}

fn json_doctor(s: &Scratch) -> serde_json::Value {
    let out = s.run(&["--json", "doctor"]);
    serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}{}", text(&out.stdout), text(&out.stderr)))["doctor"]
        .clone()
}

/// The text renderer names the moved flag and the admission lines. Kills:
/// printing the old `hooks.claude.installed: yes|no` flag, and dropping the
/// four admission lines.
#[test]
fn doctor_text_renders_claude_setup_flag_and_admission_lines() {
    let s = Scratch::new();
    let out = text(&s.run(&["doctor", "--debug"]).stdout);
    assert!(out.contains("hooks.claude.setup_installed: no\n"), "{out}");
    assert!(
        out.contains("hooks.claude.installed.admission: not_found\n"),
        "{out}"
    );
    for key in ["binary", "version", "recipe"] {
        assert!(
            out.contains(&format!("hooks.claude.installed.{key}: none\n")),
            "{out}"
        );
    }
    assert!(!out.contains("hooks.claude.installed: "), "{out}");
}

/// The built doctor fills `hooks.claude.installed` from the claude on its
/// PATH and prints one PATH line. Kills: the stub left in place for a present
/// claude, and the PATH line missing from the text report.
#[test]
fn doctor_reports_the_claude_on_path_end_to_end() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    let installed = json_doctor(&s)["hooks"]["claude"]["installed"].clone();
    assert_eq!(installed["admission"], "contract_declared", "{installed}");
    assert!(installed["version"].is_null(), "{installed}");
    assert_eq!(installed["recipe"], "claude-hooks-2.1.283", "{installed}");
    let binary = installed["binary"].as_str().unwrap().to_owned();
    assert!(binary.ends_with("/claude"), "{installed}");
    let out = text(&s.run(&["doctor", "--debug"]).stdout);
    assert!(
        out.contains(&format!("claude on PATH: {binary} (contract_declared)\n")),
        "{out}"
    );
}

/// Rewrites the installed Claude hooks into the form made before per-event registration:
/// every `--event` pair removed from the settings file and from the manifest's recorded groups,
/// with consistent fingerprints.
fn downgrade_claude_to_legacy(s: &Scratch) {
    downgrade_to_legacy(s, "claude-user", &s.settings());
}

/// The same rewrite for any harness: `kind` selects the manifest, `file` is its hook file.
fn downgrade_to_legacy(s: &Scratch, kind: &str, file: &Path) {
    use sha2::{Digest, Sha256};
    let manifest_path = fs::read_dir(s.state.join("setup"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension().is_some_and(|ext| ext == "json")
                && path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.starts_with(kind) && !name.ends_with(".created.json")
                })
        })
        .expect("manifest");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let mut settings = fs::read_to_string(file).unwrap();
    let shown = manifest.to_string();
    for entry in manifest["owned"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("no owned list in {manifest_path:?}: {shown}"))
    {
        let pair = format!(" '--event' '{}'", entry["event"].as_str().unwrap());
        assert!(settings.contains(&pair), "{pair} missing from {settings}");
        settings = settings.replace(&pair, "");
        let command = entry["group"]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .replace(&pair, "");
        entry["group"]["hooks"][0]["command"] = serde_json::json!(command);
        entry["fingerprint"] = serde_json::json!(format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&entry["group"]).unwrap())
        ));
    }
    assert!(!settings.contains("--event"));
    fs::write(file, settings.as_bytes()).unwrap();
    manifest["installed_fingerprint"] =
        serde_json::json!(format!("sha256:{:x}", Sha256::digest(settings.as_bytes())));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

/// Doctor on hooks installed before per-event registration: still installed, an informational
/// re-run-setup line and `event_registration: "legacy"`, and no limitation (so not degraded
/// because of it). A current install reports `"current"` and no line. Kills: treating a legacy
/// install as not installed (launch would refuse to start), putting the line in `limitations`,
/// and a missing or constant `event_registration`.
#[test]
fn doctor_flags_legacy_event_registration_without_degrading() {
    let s = Scratch::new();
    s.harness("claude", "2.1.286 (Claude Code)");
    fs::create_dir_all(&s.claude_config).unwrap();
    fs::write(s.settings(), b"{}").unwrap();
    let setup = s.run(&["setup", "claude"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    assert!(
        !text(&setup.stdout).contains("carry --event"),
        "a fresh install warns about nothing: {}",
        text(&setup.stdout)
    );

    let current = json_doctor(&s);
    assert_eq!(current["hooks"]["claude"]["setup"]["installed"], true);
    assert_eq!(
        current["hooks"]["claude"]["setup"]["event_registration"],
        "current"
    );
    assert!(
        !text(&s.run(&["doctor", "--debug"]).stdout).contains("predate per-event registration")
    );

    downgrade_claude_to_legacy(&s);
    let legacy = json_doctor(&s);
    assert_eq!(
        legacy["hooks"]["claude"]["setup"]["installed"], true,
        "{legacy}"
    );
    assert_eq!(
        legacy["hooks"]["claude"]["setup"]["event_registration"],
        "legacy"
    );
    assert_eq!(legacy["limitations"], current["limitations"], "{legacy}");
    assert_eq!(legacy["result"], current["result"]);
    let line = "claude hooks predate per-event registration (no --event): re-run \
                `herdr-threads setup claude`\n";
    assert!(text(&s.run(&["doctor", "--debug"]).stdout).contains(line));

    // Re-running setup rewrites the hooks with --event: doctor is back to current.
    let again = s.run(&["setup", "claude"]);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again.stderr));
    assert!(
        text(&again.stdout).contains(
            "warning: the hook commands now carry --event; a herdr-threads build from \
             before per-event registration rejects them, so to downgrade herdr-threads first \
             run `herdr-threads unsetup claude` with this build\n"
        ),
        "{}",
        text(&again.stdout)
    );
    assert!(!text(&again.stdout).contains("Codex trusts hooks by hash"));
    let after = json_doctor(&s);
    assert_eq!(after["hooks"]["claude"]["setup"]["installed"], true);
    assert_eq!(
        after["hooks"]["claude"]["setup"]["event_registration"],
        "current"
    );
    assert!(
        fs::read_to_string(s.settings())
            .unwrap()
            .contains("'--event' 'SessionStart'")
    );
}

// ---------------------------------------------------------------------------
// Claude prompt suggestions (ht-6jt). Children are never on a terminal here,
// so the bare command is the non-interactive path; the y/N question itself is
// covered in src/cli/setup.rs (`settle_prompt_suggestions`).
// ---------------------------------------------------------------------------

const SUGGESTION_KEY: &str = "promptSuggestionEnabled";

fn settings_json(s: &Scratch) -> serde_json::Value {
    serde_json::from_slice(&fs::read(s.settings()).unwrap()).unwrap()
}

/// Non-interactive `setup claude` never changes the setting: it reports
/// `advised` and prints the explanation as a warning; nothing is asked.
/// Kills: writing the key silently, or dropping the advice.
#[test]
fn non_interactive_setup_advises_and_leaves_prompt_suggestions() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    let out = s.run(&["setup", "claude"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let report = text(&out.stdout);
    assert!(
        report.contains("prompt_suggestions.action: advised\n"),
        "{report}"
    );
    assert!(
        report.contains("prompt_suggestions.state: default (enabled)\n"),
        "{report}"
    );
    assert!(
        report.contains("cannot tell that suggestion from text you typed"),
        "{report}"
    );
    assert!(report.contains("--disable-prompt-suggestions"), "{report}");
    assert!(!text(&out.stderr).contains("[y/N]"), "never asks off a TTY");
    assert!(settings_json(&s).get(SUGGESTION_KEY).is_none());
    // unsetup restores the original bytes: nothing of the setting to revert.
    let unsetup = s.run(&["--json", "unsetup", "claude"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    assert_eq!(json(&unsetup)["prompt_suggestions"], "not_recorded");
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);
}

/// `--disable-prompt-suggestions` sets the key false without asking, doctor
/// and setup-status report it, a re-run is idempotent, and unsetup reverts
/// it so the file is restored byte for byte. Kills: not writing, not
/// recording (unsetup leaving `false`), reverting before the hooks so the
/// byte-for-byte restore is lost, and doctor not reporting the state.
#[test]
fn disable_flag_sets_false_and_unsetup_reverts_it() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();

    let before = json_doctor(&s);
    assert_eq!(
        before["hooks"]["claude"]["prompt_suggestions"]["state"], "default (enabled)",
        "{before}"
    );

    let out = s.run(&["--json", "setup", "claude", "--disable-prompt-suggestions"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let report = json(&out);
    assert_eq!(
        report["prompt_suggestions"]["action"], "disabled",
        "{report}"
    );
    assert_eq!(report["prompt_suggestions"]["set_by_setup"], true);
    assert!(
        !report["warnings"]
            .to_string()
            .contains("--disable-prompt-suggestions"),
        "{report}"
    );
    let installed = settings_json(&s);
    assert_eq!(installed[SUGGESTION_KEY], false);
    assert_eq!(installed["model"], "x");
    assert!(installed["hooks"]["SessionStart"].is_array());

    let doctor = text(&s.run(&["doctor", "--debug"]).stdout);
    assert!(
        doctor.contains(&format!(
            "hooks.claude.prompt_suggestions: disabled (`{SUGGESTION_KEY}` in {}, set by setup)\n",
            s.settings().display()
        )),
        "{doctor}"
    );
    assert!(
        !doctor.contains("hooks.claude.prompt_suggestions.note"),
        "{doctor}"
    );
    let status = json(&s.run(&["--json", "setup-status", "claude"]));
    assert_eq!(status["prompt_suggestions"]["state"], "disabled");
    assert_eq!(status["prompt_suggestions"]["set_by_setup"], true);

    let again = s.run(&["--json", "setup", "claude", "--disable-prompt-suggestions"]);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again.stderr));
    assert_eq!(
        json(&again)["prompt_suggestions"]["action"],
        "already_disabled"
    );

    let unsetup = s.run(&["unsetup", "claude"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    assert!(
        text(&unsetup.stdout).contains("prompt_suggestions: reverted\n"),
        "{}",
        text(&unsetup.stdout)
    );
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);
    let after = json_doctor(&s);
    assert_eq!(
        after["hooks"]["claude"]["prompt_suggestions"]["set_by_setup"], false,
        "{after}"
    );
}

/// A settings file setup created is deleted again after the revert.
#[test]
fn disable_flag_on_a_created_settings_file_unsetup_deletes_it() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    let out = s.run(&["setup", "--disable-prompt-suggestions"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains(&format!(
            "claude: prompt suggestions: disabled (`{SUGGESTION_KEY}: false`)\n"
        )),
        "{}",
        text(&out.stdout)
    );
    assert_eq!(settings_json(&s)[SUGGESTION_KEY], false);
    let unsetup = s.run(&["unsetup"]);
    assert_eq!(unsetup.status.code(), Some(0), "{}", text(&unsetup.stderr));
    assert!(
        text(&unsetup.stdout).contains("claude: prompt suggestions: reverted"),
        "{}",
        text(&unsetup.stdout)
    );
    assert!(!s.settings().exists());
}

/// `--keep-prompt-suggestions` leaves the setting and prints no advice; a
/// value already `false` is reported and never recorded, so unsetup leaves
/// it. Kills: writing on keep, advising on keep, and reverting a value the
/// person set themselves.
#[test]
fn keep_flag_and_an_existing_false_are_left_alone() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    let out = s.run(&["--json", "setup", "claude", "--keep-prompt-suggestions"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let report = json(&out);
    assert_eq!(report["prompt_suggestions"]["action"], "kept");
    assert!(
        !report["warnings"].to_string().contains("prompt suggestion"),
        "{report}"
    );
    assert!(settings_json(&s).get(SUGGESTION_KEY).is_none());
    let unsetup = s.run(&["unsetup", "claude"]);
    assert_eq!(unsetup.status.code(), Some(0));
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);

    let own = b"{\"promptSuggestionEnabled\": false}\n";
    fs::write(s.settings(), own).unwrap();
    let out = s.run(&["--json", "setup", "claude"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let report = json(&out);
    assert_eq!(report["prompt_suggestions"]["action"], "already_disabled");
    assert_eq!(report["prompt_suggestions"]["set_by_setup"], false);
    let doctor = text(&s.run(&["doctor", "--debug"]).stdout);
    assert!(
        doctor.contains("hooks.claude.prompt_suggestions: disabled ("),
        "{doctor}"
    );
    let unsetup = s.run(&["--json", "unsetup", "claude"]);
    assert_eq!(unsetup.status.code(), Some(0));
    assert_eq!(json(&unsetup)["prompt_suggestions"], "not_recorded");
    assert_eq!(fs::read(s.settings()).unwrap(), own);
}

/// The flags belong to `setup` / `setup claude` only, and exclude each other.
#[test]
fn prompt_suggestion_flags_are_refused_elsewhere() {
    let s = Scratch::new();
    for args in [
        &["unsetup", "claude", "--disable-prompt-suggestions"][..],
        &["setup-status", "--keep-prompt-suggestions"],
        &["setup", "codex", "--disable-prompt-suggestions"],
    ] {
        let out = s.run(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(
            text(&out.stderr).contains("apply to `setup` and `setup claude` only"),
            "{args:?}: {}",
            text(&out.stderr)
        );
    }
    let both = s.run(&[
        "setup",
        "claude",
        "--disable-prompt-suggestions",
        "--keep-prompt-suggestions",
    ]);
    assert_eq!(both.status.code(), Some(2), "{}", text(&both.stderr));
}

/// Codex hooks installed before per-event registration: doctor's line adds the re-trust clause,
/// and re-running `setup codex` prints both the downgrade and the re-trust warnings (a second
/// re-run, now current, prints neither). Kills: a codex doctor line without the re-trust clause,
/// the claude line gaining it, and a warning that repeats on an already-current install.
#[test]
fn codex_legacy_registration_rerun_warns_about_retrust() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.158.0");
    fs::create_dir(&s.codex_home).unwrap();
    let setup = s.run(&["setup", "codex"]);
    assert_eq!(setup.status.code(), Some(0), "{}", text(&setup.stderr));
    assert!(!text(&setup.stdout).contains("carry --event"));

    downgrade_to_legacy(&s, "codex-user", &s.hooks());
    let doctor = text(&s.run(&["doctor", "--debug"]).stdout);
    assert!(
        doctor.contains(
            "codex hooks predate per-event registration (no --event): re-run \
             `herdr-threads setup codex`; Codex then asks to review (trust) the rewritten hooks \
             again\n"
        ),
        "{doctor}"
    );

    let again = s.run(&["setup", "codex"]);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again.stderr));
    let out = text(&again.stdout);
    assert!(
        out.contains(
            "warning: the hook commands now carry --event; a herdr-threads build from \
             before per-event registration rejects them, so to downgrade herdr-threads first \
             run `herdr-threads unsetup codex` with this build\n"
        ),
        "{out}"
    );
    assert!(
        out.contains(
            "warning: Codex trusts hooks by hash: the rewritten hook commands need \
             review again (the next interactive `codex` start, or /hooks) before Codex runs them\n"
        ),
        "{out}"
    );

    let third = text(&s.run(&["setup", "codex"]).stdout);
    assert!(!third.contains("carry --event"), "{third}");
    assert!(!third.contains("Codex trusts hooks by hash"), "{third}");
}

/// Kills operational diagnostic probes and sandbox writes through the public CLI.
#[test]
fn versionless_setup_and_status_preserve_foreign_config_without_invoking_wrapper() {
    for name in ["claude", "codex"] {
        let s = Scratch::new();
        let log = s.root.join("wrapper.log");
        let wrapper = s.bin.join(name);
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 93\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir(&s.codex_home).unwrap();
        let config = s.codex_home.join("config.toml");
        let original =
            "# foreign\nsandbox_workspace_write.network_access = false\nmodel = \"company\"\n";
        fs::write(&config, original).unwrap();
        let out = s.run(&["--json", "setup", name]);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let report = json(&out);
        assert_eq!(
            report["harness_version"]["binary"],
            wrapper.display().to_string()
        );
        assert_eq!(report["harness_version"]["admission"], "contract_declared");
        assert!(report["harness_version"]["version"].is_null());
        assert!(report["harness_version"].get("supported").is_none());
        assert_eq!(report["observed"], "unknown");
        let status = json(&s.run(&["--json", "setup-status", name]));
        assert_eq!(status["installed"], true, "{status}");
        assert_eq!(status["harness_version"]["admission"], "contract_declared");
        let alias = s.root.join("wrapper alias");
        std::os::unix::fs::symlink(&wrapper, &alias).unwrap();
        let again = s.run(&[
            "--json",
            "setup",
            name,
            "--harness-binary",
            alias.to_str().unwrap(),
        ]);
        assert_eq!(again.status.code(), Some(0), "{}", text(&again.stderr));
        assert_eq!(
            json(&again)["harness_version"]["binary"],
            alias.display().to_string()
        );
        assert!(!log.exists(), "wrapper was executed");
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
        assert_eq!(s.run(&["unsetup", name]).status.code(), Some(0));
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
    }
}

/// Kills any remaining new sandbox allowance on historically measured builds.
#[test]
fn versionless_setup_never_creates_codex_sandbox_config() {
    let s = Scratch::new();
    s.codex_with_schemas("0.159.3");
    let out = s.run(&["--json", "setup", "codex"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(s.hooks().exists());
    assert!(!s.codex_home.join("config.toml").exists());
    assert_eq!(json(&out)["created_config_file"], false);
}

/// Kills updates through incomplete/edited historical allowance ownership.
#[test]
fn versionless_setup_refuses_partial_or_edited_legacy_allowance_without_changing_hooks() {
    for prepared in [false, true] {
        let s = Scratch::new();
        s.harness("codex", "codex-cli 0.159.3");
        fs::create_dir(&s.codex_home).unwrap();
        let config = s.codex_home.join("config.toml");
        fs::write(&config, "model = \"company\"\n").unwrap();
        assert_eq!(s.run(&["setup", "codex"]).status.code(), Some(0));
        seed_legacy_allowance(&s);
        if prepared {
            let manifest =
                herdr_threads::cli::setup::manifest_path(&s.state, "codex-config", &config);
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
            value["phase"] = serde_json::json!("prepared");
            fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        } else {
            let edited = fs::read_to_string(&config)
                .unwrap()
                .replace("network_access = true", "network_access = false");
            fs::write(&config, edited).unwrap();
        }
        let hooks_before = fs::read(s.hooks()).unwrap();
        let config_before = fs::read(&config).unwrap();
        let out = s.run(&["setup", "codex"]);
        assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
        assert_eq!(fs::read(s.hooks()).unwrap(), hooks_before);
        assert_eq!(fs::read(&config).unwrap(), config_before);
        if !prepared {
            assert_eq!(s.run(&["unsetup", "codex"]).status.code(), Some(1));
            assert_eq!(fs::read(&config).unwrap(), config_before);
        }
    }
}

/// Kills declaring an explicit path available without executable metadata.
#[test]
fn versionless_setup_rejects_unusable_explicit_binary_without_config_changes() {
    let s = Scratch::new();
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), ORIGINAL).unwrap();
    let missing = s.root.join("missing");
    let directory = s.root.join("directory");
    fs::create_dir(&directory).unwrap();
    let plain = s.root.join("plain");
    fs::write(&plain, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&plain, fs::Permissions::from_mode(0o600)).unwrap();
    for path in [&missing, &directory, &plain] {
        let out = s.run(&[
            "setup",
            "claude",
            "--harness-binary",
            path.to_str().unwrap(),
        ]);
        assert_eq!(out.status.code(), Some(4), "{}", text(&out.stderr));
        assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);
        assert!(!s.state.join("setup").exists());
    }
}

/// Foreground settings remain user-owned; setup only inspects and advises.
#[test]
fn foreground_setup_and_status_inspect_both_harnesses_without_editing_flags() {
    let s = Scratch::new();
    s.harness("claude", "2.1.290");
    s.harness("codex", "0.160.1");
    fs::create_dir_all(&s.claude_config).unwrap();
    fs::create_dir_all(&s.codex_home).unwrap();
    let config = s.codex_home.join("config.toml");
    for (bytes, observed) in [
        (
            "[features]\ndaemon_auto_start = true\n",
            serde_json::json!(true),
        ),
        (
            "[features]\ndaemon_auto_start = false\n",
            serde_json::json!(false),
        ),
        ("model = \"user-choice\"\n", serde_json::Value::Null),
    ] {
        fs::write(&config, bytes).unwrap();
        let out = s.run(&["setup-status", "codex", "--json"]);
        assert!(out.status.success());
        let report = json(&out);
        assert_eq!(report["foreground"]["status"], "required");
        assert_eq!(report["foreground"]["daemon_auto_start"], observed);
        assert_eq!(fs::read(&config).unwrap(), bytes.as_bytes());
    }
    let codex_bytes = b"[features]\ndaemon_auto_start = false\n";
    fs::write(&config, codex_bytes).unwrap();
    for (setting, expected) in [
        ("{}", "required"),
        (r#"{"disableAgentView":true}"#, "configured"),
        (r#"{"disableAgentView":false}"#, "required"),
        (
            r#"{"env":{"CLAUDE_CODE_DISABLE_AGENT_VIEW":"1"}}"#,
            "configured",
        ),
        (
            r#"{"env":{"CLAUDE_CODE_DISABLE_AGENT_VIEW":" TRUE "}}"#,
            "configured",
        ),
    ] {
        fs::write(s.settings(), setting).unwrap();
        for verb in ["setup-status", "setup"] {
            let mut args = vec![verb, "claude", "--json"];
            if verb == "setup" {
                args.push("--keep-prompt-suggestions");
            }
            let out = s.run(&args);
            assert!(out.status.success(), "{}", text(&out.stderr));
            let report = json(&out);
            assert_eq!(report["foreground"]["status"], expected);
            if expected == "configured" {
                assert!(
                    !report["warnings"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|warning| warning
                            .as_str()
                            .is_some_and(|w| w.starts_with("Foreground user settings")))
                );
            }
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(s.settings()).unwrap()).unwrap();
            let original: serde_json::Value = serde_json::from_str(setting).unwrap();
            assert_eq!(
                value.get("disableAgentView"),
                original.get("disableAgentView")
            );
            // The only `env` change is the delivery mod's own key (ht-j16.7).
            let mut env = value.get("env").cloned();
            if let Some(serde_json::Value::Object(map)) = &mut env {
                map.remove(PLUGIN_DIRS);
                if map.is_empty() && original.get("env").is_none() {
                    env = None;
                }
            }
            assert_eq!(env.as_ref(), original.get("env"));
            let codex = s.run(&[verb, "codex", "--json"]);
            assert!(codex.status.success(), "{}", text(&codex.stderr));
            let codex = json(&codex);
            assert_eq!(codex["foreground"]["status"], "required");
            assert!(
                codex["foreground"]["advice"]
                    .as_str()
                    .unwrap()
                    .contains("--no-daemon")
            );
            assert_eq!(fs::read(&config).unwrap(), codex_bytes);
        }
        let removed = s.run(&["unsetup", "claude"]);
        assert!(removed.status.success(), "{}", text(&removed.stderr));
    }
    for verb in ["setup-status", "setup"] {
        let mut args = vec![verb, "--json"];
        if verb == "setup" {
            args.push("--keep-prompt-suggestions");
        }
        let all = s.run(&args);
        assert!(all.status.success());
        for entry in json(&all)["harnesses"].as_array().unwrap() {
            assert!(entry["report"]["foreground"].is_object());
            let warnings = entry["report"]["warnings"].as_array().unwrap();
            let mut unique = warnings.iter().map(|v| v.to_string()).collect::<Vec<_>>();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), warnings.len());
        }
    }
    for args in [
        vec!["setup-status"],
        vec!["setup", "--keep-prompt-suggestions"],
        vec!["setup-status", "codex"],
    ] {
        let out = s.run(&args);
        assert!(out.status.success());
        let out = text(&out.stdout);
        assert!(out.contains("--no-daemon"), "{out}");
        if args.len() == 1 || args[0] == "setup" {
            assert!(out.contains("disableAgentView"), "{out}");
        }
    }
}

#[test]
fn foreground_status_missing_and_malformed_user_files_are_honest_and_read_only() {
    for harness in ["claude", "codex"] {
        let s = Scratch::new();
        s.harness(harness, "wrapper-version-unknown");
        let out = s.run(&["setup-status", harness, "--json"]);
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert_eq!(json(&out)["foreground"]["status"], "required");
        let path = if harness == "claude" {
            s.settings()
        } else {
            s.codex_home.join("config.toml")
        };
        assert!(!path.exists());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "invalid {{").unwrap();
        let out = s.run(&["setup-status", harness, "--json"]);
        assert!(out.status.success(), "{}", text(&out.stderr));
        let report = json(&out);
        assert_eq!(report["foreground"]["status"], "unknown");
        assert!(report["foreground"]["inspection_error"].is_string());
        assert_eq!(fs::read(&path).unwrap(), b"invalid {{");
        let out = s.run(&["setup-status", harness]);
        assert!(out.status.success());
        assert!(text(&out.stdout).contains("unknown"));
    }
}

/// Kills deleting the launch entrypoint's configured-option resolution: with
/// no daemon or host socket, the specific quoting error must still be reported.
#[test]
fn launch_entrypoint_rejects_malformed_configured_options_before_connect() {
    use herdr_threads::test_support::spawn::SpawnOwned;
    for (harness, variable) in [
        ("codex", "HERDR_THREADS_CODEX_OPTS"),
        ("claude", "HERDR_THREADS_CLAUDE_OPTS"),
    ] {
        let s = Scratch::new();
        let mut command = s.command(&s.root);
        herdr_threads::test_support::spawn::tag(&mut command);
        command
            .arg("--state-dir")
            .arg(&s.state)
            .arg("--host-endpoint")
            .arg(s.host())
            .args(["launch", "--pane", "w1:p2", "--kind", harness])
            .env(variable, "'unterminated")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let output = command.spawn_owned().unwrap().wait_with_output().unwrap();
        let stderr = text(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{harness}: {stderr}");
        assert!(
            stderr.contains(&format!("{variable} has invalid argument quoting")),
            "{harness}: {stderr}"
        );
        assert!(
            output.stdout.is_empty(),
            "{harness}: {}",
            text(&output.stdout)
        );
        assert!(!s.state.exists(), "{harness}: launch created durable state");
        assert!(
            !s.host().exists(),
            "{harness}: launch created a host socket"
        );
        assert!(
            !s.settings().exists(),
            "{harness}: launch wrote Claude settings"
        );
        assert!(!s.hooks().exists(), "{harness}: launch wrote Codex hooks");
    }
}

// ------------------------------------------------------- delivery mod (ht-j16.7)

const PLUGIN_DIRS: &str = "CLAUDE_CODE_PLUGIN_DIRS";

fn claude_scratch(settings: &[u8]) -> Scratch {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)");
    fs::create_dir(&s.claude_config).unwrap();
    fs::write(s.settings(), settings).unwrap();
    s
}

fn plugin_dirs(s: &Scratch) -> Option<String> {
    settings_json(s)["env"][PLUGIN_DIRS]
        .as_str()
        .map(str::to_owned)
}

fn mod_records(s: &Scratch) -> Vec<String> {
    fs::read_dir(s.state.join("setup"))
        .map(|dir| {
            dir.map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with("claude-mod-"))
                .collect()
        })
        .unwrap_or_default()
}

/// Kills: a setup that skips the mod, writes a partial file set, or leaves
/// `setup-status` blind to it; a status that claims a daemon channel count
/// without a daemon.
#[test]
fn setup_claude_installs_mod_and_status_reports_it() {
    let s = claude_scratch(ORIGINAL);
    let out = s.run(&["--json", "setup", "claude"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let report = json(&out);
    assert_eq!(report["mod"]["action"], "installed", "{report}");
    let dir = s.mod_dir();
    assert_eq!(report["mod"]["dir"], dir.display().to_string());
    for file in [
        ".claude-plugin/plugin.json",
        "hooks/hooks.json",
        "hooks/register.js",
        "types/index.d.ts",
    ] {
        assert!(dir.join(file).is_file(), "{file}");
    }
    assert!(!dir.join("README.md").exists());
    assert_eq!(plugin_dirs(&s), Some(dir.display().to_string()));
    assert_eq!(settings_json(&s)["model"], "x");
    assert_eq!(mod_records(&s).len(), 1);

    let status = json(&s.run(&["--json", "setup-status", "claude"]));
    let delivery = &status["mod"];
    assert_eq!(delivery["installed"], true, "{status}");
    assert_eq!(delivery["settings_value_contains_mod_dir"], true);
    assert_eq!(delivery["managed_policy"]["state"], "none");
    assert_eq!(delivery["shell_env"], serde_json::Value::Null);
    assert_eq!(
        delivery["claude_version_supported"],
        serde_json::Value::Null
    );
    assert_eq!(delivery["daemon"]["status"], "channel status unavailable");
    assert_eq!(
        delivery["note"],
        "an install does not prove any session loaded the mod"
    );
    assert!(delivery.get("session_override").is_none());

    // The text report carries one `mod:` line.
    let text_status = text(&s.run(&["setup-status", "claude"]).stdout);
    assert!(
        text_status.contains("mod.installed: true\n"),
        "{text_status}"
    );
    let bare = text(&s.run(&["setup"]).stdout);
    assert!(bare.contains("claude: mod: already_installed\n"), "{bare}");
    let bare_status = text(&s.run(&["setup-status"]).stdout);
    assert!(
        bare_status.contains("claude: mod: installed; ")
            && bare_status.contains("daemon: channel status unavailable")
            && bare_status.contains("an install does not prove any session loaded the mod"),
        "{bare_status}"
    );
}

#[test]
fn setup_status_channel_status_unavailable_without_daemon() {
    let s = claude_scratch(ORIGINAL);
    let status = json(&s.run(&["--json", "setup-status", "claude"]));
    assert_eq!(status["mod"]["installed"], false, "{status}");
    assert_eq!(status["mod"]["files_present"], false);
    assert_eq!(
        status["mod"]["daemon"]["status"],
        "channel status unavailable"
    );
    // Status never starts a daemon or writes anything.
    assert!(!s.mod_dir().exists());
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);
    assert!(
        !s.state.join("daemon").exists() && !s.host().exists(),
        "status created daemon state"
    );
}

#[test]
fn setup_claude_preserves_existing_plugin_dirs() {
    let s = claude_scratch(br#"{"env":{"CLAUDE_CODE_PLUGIN_DIRS":"/opt/a:/opt/b","KEEP":"1"}}"#);
    assert_eq!(s.run(&["setup", "claude"]).status.code(), Some(0));
    assert_eq!(
        plugin_dirs(&s),
        Some(format!("/opt/a:/opt/b:{}", s.mod_dir().display()))
    );
    assert_eq!(settings_json(&s)["env"]["KEEP"], "1");
}

#[test]
fn setup_claude_twice_is_idempotent() {
    let s = claude_scratch(ORIGINAL);
    assert_eq!(s.run(&["setup", "claude"]).status.code(), Some(0));
    let settings = fs::read(s.settings()).unwrap();
    let register = s.mod_dir().join("hooks/register.js");
    let before = fs::metadata(&register).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    let again = json(&s.run(&["--json", "setup", "claude"]));
    assert_eq!(again["mod"]["action"], "already_installed");
    assert_eq!(again["mod"]["files_written"], serde_json::json!([]));
    assert_eq!(fs::read(s.settings()).unwrap(), settings);
    assert_eq!(fs::metadata(&register).unwrap().modified().unwrap(), before);
    assert_eq!(mod_records(&s).len(), 1);

    // Upgrade: a stale file is rewritten, the settings value is not.
    fs::write(&register, b"// older").unwrap();
    let upgraded = json(&s.run(&["--json", "setup", "claude"]));
    assert_eq!(upgraded["mod"]["action"], "upgraded");
    assert_eq!(
        upgraded["mod"]["files_written"],
        serde_json::json!(["hooks/register.js"])
    );
    assert_ne!(fs::read(&register).unwrap(), b"// older");
    assert_eq!(fs::read(s.settings()).unwrap(), settings);
}

#[test]
fn unsetup_claude_removes_mod_path_files_and_manifest() {
    let s = claude_scratch(br#"{"env":{"CLAUDE_CODE_PLUGIN_DIRS":"/opt/a"}}"#);
    assert_eq!(s.run(&["setup", "claude"]).status.code(), Some(0));
    assert!(s.mod_dir().is_dir());
    let removed = json(&s.run(&["--json", "unsetup", "claude"]));
    assert_eq!(removed["mod"], "reverted", "{removed}");
    assert_eq!(removed["action"], "removed");
    assert_eq!(plugin_dirs(&s), Some("/opt/a".to_owned()));
    assert!(!s.mod_dir().exists());
    assert!(mod_records(&s).is_empty());

    // Created from nothing: unsetup leaves the original bytes.
    let s = claude_scratch(ORIGINAL);
    assert_eq!(s.run(&["setup", "claude"]).status.code(), Some(0));
    assert_eq!(s.run(&["unsetup", "claude"]).status.code(), Some(0));
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);
    assert!(!s.mod_dir().exists());
    assert!(mod_records(&s).is_empty());
}

#[test]
fn setup_claude_hooks_only_removes_mod_path() {
    let s = claude_scratch(ORIGINAL);
    assert_eq!(s.run(&["setup", "claude"]).status.code(), Some(0));
    assert!(plugin_dirs(&s).is_some());
    let hooks_only = json(&s.run(&["--json", "setup", "claude", "--hooks-only"]));
    assert_eq!(hooks_only["mod"]["action"], "removed_hooks_only");
    assert_eq!(hooks_only["mod"]["settings_entry"], "reverted");
    assert_eq!(plugin_dirs(&s), None);
    assert!(settings_json(&s).get("env").is_none());
    assert!(settings_json(&s)["hooks"]["SessionStart"].is_array());
    assert!(!s.mod_dir().exists());
    assert!(mod_records(&s).is_empty());
    // The hooks stay owned: unsetup still restores the original bytes.
    assert_eq!(s.run(&["unsetup", "claude"]).status.code(), Some(0));
    assert_eq!(fs::read(s.settings()).unwrap(), ORIGINAL);

    // On a fresh machine --hooks-only installs hooks and no mod.
    let fresh = claude_scratch(ORIGINAL);
    let report = json(&fresh.run(&["--json", "setup", "claude", "--hooks-only"]));
    assert_eq!(report["action"], "installed");
    assert_eq!(report["mod"]["settings_entry"], "not_recorded");
    assert_eq!(plugin_dirs(&fresh), None);
    assert!(!fresh.mod_dir().exists());

    // Claude only, and only for setup.
    for args in [
        &["setup", "--hooks-only"][..],
        &["setup", "codex", "--hooks-only"],
        &["unsetup", "claude", "--hooks-only"],
        &["setup-status", "claude", "--hooks-only"],
    ] {
        let out = fresh.run(args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?}: {}",
            text(&out.stderr)
        );
        assert!(text(&out.stderr).contains("--hooks-only"), "{args:?}");
    }
}

fn managed_policy(s: &Scratch, content: &str) -> PathBuf {
    let file = s.root.join("managed-settings.json");
    fs::write(&file, content).unwrap();
    file
}

#[test]
fn setup_status_reports_managed_policy_and_offers_hooks_only() {
    let s = claude_scratch(ORIGINAL);
    let managed = managed_policy(&s, r#"{"disableSideloadFlags": true}"#);
    let vars = [(
        herdr_threads::harness::claude_mod::TEST_MANAGED_SETTINGS_ENV,
        managed.as_os_str(),
    )];

    // A fresh install under the policy is hooks-only and says so.
    let out = s.run_with(&vars, &["--json", "setup", "claude"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let report = json(&out);
    assert_eq!(report["mod"]["action"], "skipped_managed_policy");
    assert_eq!(
        report["mod"]["managed_policy"],
        serde_json::json!({"state": "disable_side_load_flags", "source": managed.display().to_string()})
    );
    assert!(
        report["warnings"]
            .to_string()
            .contains("the delivery mod was not installed"),
        "{report}"
    );
    assert_eq!(plugin_dirs(&s), None);
    assert!(!s.mod_dir().exists());
    assert!(mod_records(&s).is_empty());

    // The policy appears after an install: status and re-runs say Claude
    // Code will refuse to start and offer --hooks-only.
    assert_eq!(s.run(&["setup", "claude"]).status.code(), Some(0));
    assert!(plugin_dirs(&s).is_some());
    let status = json(&s.run_with(&vars, &["--json", "setup-status", "claude"]));
    assert_eq!(
        status["mod"]["managed_policy"]["state"],
        "disable_side_load_flags"
    );
    let advice = status["mod"]["advice"].as_str().unwrap();
    assert!(
        advice.contains("will refuse to start")
            && advice.contains("herdr-threads setup claude --hooks-only"),
        "{advice}"
    );
    let rerun = json(&s.run_with(&vars, &["--json", "setup", "claude"]));
    assert_eq!(rerun["mod"]["action"], "skipped_managed_policy");
    assert!(
        rerun["warnings"]
            .to_string()
            .contains("setup claude --hooks-only"),
        "{rerun}"
    );
    assert!(plugin_dirs(&s).is_some());
    let text_status = text(&s.run_with(&vars, &["setup-status"]).stdout);
    assert!(text_status.contains("blocks it"), "{text_status}");

    let fixed = json(&s.run_with(&vars, &["--json", "setup", "claude", "--hooks-only"]));
    assert_eq!(fixed["mod"]["settings_entry"], "reverted");
    assert_eq!(plugin_dirs(&s), None);
    assert!(!s.mod_dir().exists());
}

#[test]
fn setup_status_reports_shell_env_and_session_override() {
    let s = claude_scratch(br#"{"env":{"CLAUDE_CODE_PLUGIN_DIRS":"/a"}}"#);
    let shell = std::ffi::OsString::from("/s1:/a:/s2");
    let off = std::ffi::OsString::from("off");
    let vars = [
        (PLUGIN_DIRS, shell.as_os_str()),
        (
            herdr_threads::protocol::watch::MOD_DELIVERY_ENV,
            off.as_os_str(),
        ),
    ];
    let out = s.run_with(&vars, &["--json", "setup", "claude"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let report = json(&out);
    // Non-interactive: the shell's directories are carried over, once.
    assert_eq!(
        plugin_dirs(&s),
        Some(format!("/a:/s1:/s2:{}", s.mod_dir().display()))
    );
    assert_eq!(
        report["mod"]["shell_env"]["copied"],
        serde_json::json!(["/s1", "/s2"])
    );
    assert!(
        report["warnings"]
            .to_string()
            .contains("a settings `env` value replaces it"),
        "{report}"
    );

    let status = json(&s.run_with(&vars, &["--json", "setup-status", "claude"]));
    assert_eq!(status["mod"]["shell_env"], "/s1:/a:/s2");
    assert_eq!(status["mod"]["session_override"], "off");
    // The daemon-side switch is reported by the daemon, not by this env.
    assert_eq!(
        status["mod"]["daemon"]["status"],
        "channel status unavailable"
    );
    let plain = json(&s.run(&["--json", "setup-status", "claude"]));
    assert!(plain["mod"].get("session_override").is_none());
    assert_eq!(plain["mod"]["shell_env"], serde_json::Value::Null);

    // unsetup removes the appended path and the copied directories.
    assert_eq!(s.run(&["unsetup", "claude"]).status.code(), Some(0));
    assert_eq!(plugin_dirs(&s), Some("/a".to_owned()));
}
