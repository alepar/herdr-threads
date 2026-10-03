//! Package lifecycle and CLI UX through the built executable: help/version,
//! human-readable errors with stable exit statuses, long host endpoint
//! selectors, Herdr-style 0755 state roots, degraded-but-reachable ensure,
//! and a real doctor report. Each test names the mutation it kills.

use herdr_threads::test_support::spawn::SpawnOwned;
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

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = PathBuf::from(format!(
            "/private/tmp/htux-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self(root)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Stops the daemon for a state/host context even when an assertion fails.
struct DaemonGuard {
    state: PathBuf,
    host: PathBuf,
}
impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = run(&self.state, &self.host, &["daemon", "stop"], None);
    }
}

fn run(state: &Path, host: &Path, args: &[&str], cwd: Option<&Path>) -> Output {
    run_in_pane(state, host, None, args, cwd)
}

/// Like [`run`], with `HERDR_PANE_ID` set to `pane` when one is given (and
/// always removed otherwise, so an inherited pane never leaks in).
fn run_in_pane(
    state: &Path,
    host: &Path,
    pane: Option<&str>,
    args: &[&str],
    cwd: Option<&Path>,
) -> Output {
    let mut command = scrubbed_command(BIN);
    command
        .arg("--state-dir")
        .arg(state)
        .arg("--host-endpoint")
        .arg(host)
        .args(args)
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_BIN_PATH");
    scratch_homes(&mut command, state);
    if let Some(pane) = pane {
        command.env("HERDR_PANE_ID", pane);
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.output().unwrap()
}

/// Like [`run`] with `PATH` replaced by `path` (a daemon this starts inherits
/// it, so its harness observation sees only what `path` holds).
fn run_with_path(state: &Path, host: &Path, args: &[&str], path: &Path) -> Output {
    let mut command = scrubbed_command(BIN);
    command
        .arg("--state-dir")
        .arg(state)
        .arg("--host-endpoint")
        .arg(host)
        .args(args)
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_BIN_PATH")
        .env("PATH", path);
    scratch_homes(&mut command, state);
    command.output().unwrap()
}

/// A scratch `PATH` directory holding a synthetic `claude` reporting
/// `version`.
fn claude_on_path(root: &Path, version: &str) -> PathBuf {
    let dir = root.join("bin-claude");
    fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let claude = dir.join("claude");
    fs::write(
        &claude,
        format!("#!/bin/sh\nprintf '{version} (Claude Code)\\n'\nexit 0\n"),
    )
    .unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// Point every harness config home at the scratch root holding `state`, so
/// no test reads (or writes) the invoking user's ~/.claude or ~/.codex.
fn scratch_homes(command: &mut Command, state: &Path) {
    let root = state.parent().unwrap();
    command
        .env("HOME", root.join("home"))
        .env("CLAUDE_CONFIG_DIR", root.join("claude-config"))
        .env("CODEX_HOME", root.join("codex-home"))
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME");
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A host endpoint path longer than the 128-byte opaque-ID limit, with spaces.
fn long_host(root: &Path) -> PathBuf {
    let dir = root.join("a directory with spaces that is deliberately long for host sockets");
    fs::create_dir_all(&dir).unwrap();
    let host = dir.join("herdr host socket with a long file name.sock");
    assert!(host.as_os_str().len() > 128, "{}", host.display());
    host
}

/// Kills: mapping clap DisplayHelp/DisplayVersion to InvalidRequest (exit 2,
/// Debug struct on stderr) and removing `#[command(version)]`.
#[test]
fn help_and_version_exit_zero_with_plain_stdout() {
    let help = scrubbed_command(BIN).arg("--help").output().unwrap();
    assert_eq!(help.status.code(), Some(0), "{}", text(&help.stderr));
    assert!(help.stderr.is_empty(), "{}", text(&help.stderr));
    let stdout = text(&help.stdout);
    assert!(stdout.contains("Usage:"), "{stdout}");
    assert!(stdout.contains("Exit status:"), "{stdout}");
    assert!(!stdout.contains("ApiError"), "{stdout}");

    let sub = scrubbed_command(BIN)
        .args(["daemon", "--help"])
        .output()
        .unwrap();
    assert_eq!(sub.status.code(), Some(0));
    assert!(text(&sub.stdout).contains("ensure"));

    let version = scrubbed_command(BIN).arg("--version").output().unwrap();
    assert_eq!(version.status.code(), Some(0), "{}", text(&version.stderr));
    assert_eq!(
        text(&version.stdout),
        format!("herdr-threads {}\n", env!("CARGO_PKG_VERSION"))
    );
}

/// Kills: printing `{error:?}` in main, and a constant exit status for every
/// error (the previous behavior was always 2).
#[test]
fn errors_are_human_readable_with_stable_exit_statuses() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");

    let usage = run(&state, &host, &["thread", "--bogus-flag"], None);
    assert_eq!(usage.status.code(), Some(2));
    let stderr = text(&usage.stderr);
    assert!(stderr.starts_with("herdr-threads: error:"), "{stderr}");
    assert!(stderr.contains("(invalid_request)"), "{stderr}");
    assert!(!stderr.contains("ApiError"), "{stderr}");

    let unavailable = run(&state, &host, &["daemon", "health"], None);
    assert_eq!(
        unavailable.status.code(),
        Some(3),
        "{}",
        text(&unavailable.stderr)
    );
    let stderr = text(&unavailable.stderr);
    assert!(
        stderr.contains("daemon is not running") && stderr.contains("daemon ensure"),
        "{stderr}"
    );
    assert!(stderr.contains("(host_unavailable)"), "{stderr}");
    assert!(!stderr.contains("Api("), "{stderr}");

    let unsupported = run(&state, &host, &["view"], None);
    assert_eq!(
        unsupported.status.code(),
        Some(4),
        "{}",
        text(&unsupported.stderr)
    );
    assert!(text(&unsupported.stderr).contains("(unsupported)"));

    // No flag, no env, no Herdr default and no `herdr` on PATH: refused.
    let missing_context = plain_shell(&scratch.0)
        .args(["daemon", "health"])
        .output()
        .unwrap();
    assert_eq!(missing_context.status.code(), Some(2));
    assert!(text(&missing_context.stderr).contains("--state-dir"));
}

/// A plain shell (not a Herdr pane): no Herdr env, scratch HOME, no XDG
/// overrides and no `herdr` on PATH, so only Herdr's default locations under
/// the scratch HOME can name the instance.
fn plain_shell(root: &Path) -> Command {
    let mut command = scrubbed_command(BIN);
    command
        .current_dir(root)
        .env("HOME", root.join("home"))
        .env("PATH", "/usr/bin:/bin")
        .env("CLAUDE_CONFIG_DIR", root.join("claude-config"))
        .env("CODEX_HOME", root.join("codex-home"))
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HERDR_SESSION")
        .env_remove("HERDR_CONFIG_PATH")
        .env_remove("HERDR_ENV")
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_BIN_PATH");
    command
}

/// ht-4is.8.14: every command, not only setup, finds the Herdr instance
/// from Herdr's default locations when no flag or env names it.
/// Kills: `doctor` (and other commands) printing "state directory missing"
/// in a plain shell where Herdr's plugin state root and socket exist.
#[test]
fn plain_shell_commands_resolve_herdr_default_instance() {
    let scratch = Scratch::new();
    let home = scratch.0.join("home");
    let state = home.join(".local/state/herdr/plugins/herdr-threads");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&state)
        .unwrap();
    fs::create_dir_all(home.join(".config/herdr")).unwrap();
    let socket = home.join(".config/herdr/herdr.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let _guard = DaemonGuard {
        state: state.clone(),
        host: socket.clone(),
    };

    let doctor = plain_shell(&scratch.0)
        .args(["--json", "doctor"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&doctor.stdout)
        .unwrap_or_else(|_| panic!("{}", text(&doctor.stderr)));
    let context = &value["doctor"]["context"];
    assert_eq!(context["ok"], true, "{value}");
    assert_eq!(context["state_dir"], state.display().to_string());
    assert_eq!(context["host_endpoint"], socket.display().to_string());
    assert_eq!(context["source"]["state_dir"], "default (~/.local/state)");
    assert_eq!(context["source"]["host_endpoint"], "default (~/.config)");

    // An ordinary command resolves the same instance: it reaches the
    // daemon stage (not running: status 3), never a context error (2).
    let health = plain_shell(&scratch.0)
        .args(["daemon", "health"])
        .output()
        .unwrap();
    assert_ne!(health.status.code(), Some(2), "{}", text(&health.stderr));
    assert!(
        !text(&health.stderr).contains("state directory"),
        "{}",
        text(&health.stderr)
    );

    // A second state root without a store is a stale leftover: XDG wins, nothing is ambiguous.
    let xdg_state = scratch.0.join("xdg/herdr/plugins/herdr-threads");
    fs::create_dir_all(&xdg_state).unwrap();
    let stale = plain_shell(&scratch.0)
        .env("XDG_STATE_HOME", scratch.0.join("xdg"))
        .args(["--json", "doctor"])
        .output()
        .unwrap();
    let stale: serde_json::Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(
        stale["doctor"]["context"]["source"]["state_dir"], "default (XDG_STATE_HOME)",
        "{stale}"
    );
    // Both holding a store makes the default ambiguous: refused, not picked.
    for root in [&state, &xdg_state] {
        fs::create_dir_all(root.join("instances/abc")).unwrap();
        fs::write(root.join("instances/abc/threads.sqlite3"), "").unwrap();
    }
    let ambiguous = plain_shell(&scratch.0)
        .env("XDG_STATE_HOME", scratch.0.join("xdg"))
        .args(["daemon", "health"])
        .output()
        .unwrap();
    assert_eq!(ambiguous.status.code(), Some(2));
    assert!(
        text(&ambiguous.stderr).contains("both"),
        "{}",
        text(&ambiguous.stderr)
    );
}

/// Kills: parsing `--host-endpoint` as a 128-byte ASCII opaque ID (every
/// command failed with "invalid opaque id" before reaching the daemon).
#[test]
fn long_host_endpoint_path_is_a_valid_selector() {
    let scratch = Scratch::new();
    let host = long_host(&scratch.0);
    let output = run(&scratch.0.join("state"), &host, &["daemon", "health"], None);
    let stderr = text(&output.stderr);
    assert!(!stderr.contains("opaque id"), "{stderr}");
    assert_eq!(output.status.code(), Some(3), "{stderr}");
}

/// Kills: requiring the Herdr-supplied state root itself to be 0700 (in
/// paths or in service config load), and failing `daemon ensure` when the
/// reachable daemon reports Degraded health.
#[test]
fn ensure_accepts_herdr_0755_state_root_and_degraded_daemon() {
    let scratch = Scratch::new();
    let state = scratch.0.join("plugin-state");
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    let host = long_host(&scratch.0);
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };

    let ensure = run(&state, &host, &["daemon", "ensure"], None);
    assert_eq!(
        ensure.status.code(),
        Some(0),
        "stdout={} stderr={}",
        text(&ensure.stdout),
        text(&ensure.stderr)
    );
    assert!(
        text(&ensure.stdout).contains("degraded"),
        "{}",
        text(&ensure.stdout)
    );
    let instances = fs::metadata(state.join("instances")).unwrap();
    assert_eq!(instances.permissions().mode() & 0o777, 0o700);
    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o755,
        "the host-owned root is not rewritten"
    );

    let again = run(&state, &host, &["daemon", "ensure"], None);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again.stderr));

    let stop = run(&state, &host, &["daemon", "stop"], None);
    assert_eq!(stop.status.code(), Some(0), "{}", text(&stop.stderr));
    assert!(text(&stop.stdout).contains("stop_accepted"));
}

/// Kills: a shutdown join that waits out the hung `--version` runs, so stop
/// answers DeadlineExceeded (final review S6), and a hung harness child left
/// running after the daemon exits.
#[test]
fn daemon_stop_completes_with_a_hung_harness_binary() {
    let scratch = Scratch::new();
    let state = scratch.0.join("plugin-state");
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    let host = long_host(&scratch.0);
    let bin = scratch.0.join("bin-hung");
    fs::DirBuilder::new().mode(0o700).create(&bin).unwrap();
    let marker = |name: &str| scratch.0.join(format!("{name}.pid"));
    for name in ["claude", "codex"] {
        let path = bin.join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\necho $$ > '{0}.tmp'\n/bin/mv '{0}.tmp' '{0}'\nexec /bin/sleep 60\n",
                marker(name).display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };

    let ensure = run_with_path(&state, &host, &["daemon", "ensure"], &bin);
    assert_eq!(
        ensure.status.code(),
        Some(0),
        "stdout={} stderr={}",
        text(&ensure.stdout),
        text(&ensure.stderr)
    );
    let started = std::time::Instant::now();
    while !marker("claude").exists() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "the admission observer never started claude --version"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let stop = run(&state, &host, &["daemon", "stop"], None);
    assert_eq!(
        stop.status.code(),
        Some(0),
        "stdout={} stderr={}",
        text(&stop.stdout),
        text(&stop.stderr)
    );
    assert!(text(&stop.stdout).contains("stop_accepted"));

    let alive = |pid: &str| {
        Command::new("kill")
            .args(["-0", pid])
            .status()
            .unwrap()
            .success()
    };
    for name in ["claude", "codex"] {
        let Ok(pid) = fs::read_to_string(marker(name)) else {
            continue;
        };
        let pid = pid.trim().to_owned();
        let started = std::time::Instant::now();
        while alive(&pid) && started.elapsed() < std::time::Duration::from_secs(2) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!alive(&pid), "hung {name} child {pid} outlived the daemon");
    }
}

/// Kills: accepting any state root mode (dropping the group/other write check).
#[test]
fn ensure_rejects_group_writable_state_root() {
    let scratch = Scratch::new();
    let state = scratch.0.join("shared-state");
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o775)).unwrap();
    let host = scratch.0.join("host.sock");
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };
    let ensure = run(&state, &host, &["daemon", "ensure"], None);
    assert_ne!(ensure.status.code(), Some(0));
    assert!(
        text(&ensure.stderr).contains("unsafe state directory"),
        "{}",
        text(&ensure.stderr)
    );
    assert!(!state.join("instances").exists());
}

/// Kills: the former hard-coded `Unsupported` doctor, a doctor that ignores
/// daemon reachability, a doctor that never inspects the owned user-level
/// Claude installation (in the CLAUDE_CONFIG_DIR it resolves), and a doctor
/// that stops listing the adapter recipe registries (supported versions per
/// recipe).
#[test]
fn doctor_reports_context_daemon_and_owned_hook_installation() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let config = scratch.0.join("claude-config");
    fs::create_dir_all(&config).unwrap();
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };

    let down = run(&state, &host, &["doctor", "--debug"], None);
    assert_eq!(down.status.code(), Some(3), "{}", text(&down.stderr));
    let report = text(&down.stdout);
    assert!(
        report.contains(&format!("version: {}", env!("CARGO_PKG_VERSION"))),
        "{report}"
    );
    assert!(report.contains("daemon: not_running"), "{report}");
    assert!(
        report.contains("hooks.claude.setup_installed: no"),
        "{report}"
    );
    assert!(
        report.contains(&format!(
            "hooks.claude.settings: {}\n",
            config.join("settings.json").display()
        )),
        "{report}"
    );
    assert!(
        report.contains("hooks.codex.setup_installed: no"),
        "{report}"
    );
    assert!(
        report.contains(
            "hooks.claude.recipes: claude-hooks-2.1.283 [2.1.283, 2.1.286]; claude-hooks-2.1.287 {2.1.287}\n"
        ),
        "{report}"
    );
    assert!(
        report.contains(
            "hooks.claude.compaction_recovery: claude-hooks-2.1.283: unsupported (resume/clear and herdr-threads summary); claude-hooks-2.1.287: supported\n"
        ),
        "{report}"
    );
    assert!(
        report.contains("skill.summary_procedure: present\n"),
        "{report}"
    );
    assert!(
        report.contains("hooks.codex.recipes: codex-hooks-v1 {0.157.1, 0.158.0, 0.159.3}\n"),
        "{report}"
    );
    assert!(report.contains("result: unavailable"), "{report}");
    assert!(down.stderr.is_empty(), "{}", text(&down.stderr));

    // The daemon observes the harnesses on its own PATH: an admitted
    // `claude` and no `codex`.
    let path = claude_on_path(&scratch.0, "2.1.286");
    let ensure = run_with_path(&state, &host, &["daemon", "ensure"], &path);
    assert_eq!(ensure.status.code(), Some(0), "{}", text(&ensure.stderr));

    let settings = config.join("settings.json");
    fs::write(&settings, b"{}").unwrap();
    // claude is on PATH but its hooks are not installed: a doctor limitation.
    let missing = run_with_path(&state, &host, &["doctor", "--debug"], &path);
    let report = text(&missing.stdout);
    assert!(
        report.contains(&format!(
            "limitation: claude is on PATH ({}) but its hooks are not installed in {}: run \
             `herdr-threads setup claude`\n",
            path.join("claude").display(),
            settings.display()
        )),
        "{report}"
    );
    let manifest = herdr_threads::cli::setup::manifest_path(&state, "claude-user", &settings);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(manifest.parent().unwrap())
        .unwrap();
    let kind = herdr_threads::harness::setup::SettingsKind::ClaudeUser;
    herdr_threads::harness::setup::install_user_settings(
        kind,
        &settings,
        &manifest,
        &[BIN.to_string(), "hook".into(), "claude".into()],
        b"{}",
    )
    .unwrap();

    // The boot observation is bounded and asynchronous: wait for it.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let value = loop {
        let json = run_with_path(&state, &host, &["--json", "doctor"], &path);
        assert_eq!(json.status.code(), Some(0));
        let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
        if value["doctor"]["daemon"]["harness_codex"] != "unknown"
            && value["doctor"]["daemon"]["harness_claude"] != "unknown"
            || std::time::Instant::now() >= until
        {
            break value;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(
        value["doctor"]["hooks"]["claude"]["setup"]["installed"],
        true
    );
    assert_eq!(value["doctor"]["hooks"]["claude"]["scope"], "user");
    assert_eq!(
        value["doctor"]["hooks"]["codex"]["recipes"],
        "codex-hooks-v1 {0.157.1, 0.158.0, 0.159.3}"
    );
    assert_eq!(
        value["doctor"]["skill"],
        serde_json::json!({"summary_procedure": "present"})
    );
    // Cooperative is the designed mode, distinct from unsupported (absent).
    assert_eq!(value["doctor"]["daemon"]["harness_claude"], "cooperative");
    assert_eq!(value["doctor"]["daemon"]["harness_codex"], "unsupported");
    // The host socket is missing: a real problem, so still degraded.
    assert_eq!(value["doctor"]["daemon"]["state"], "degraded");
    assert_eq!(value["doctor"]["limitations"], serde_json::json!([]));
    let notes = value["doctor"]["daemon"]["notes"].as_array().unwrap();
    assert!(
        notes.contains(&serde_json::json!(
            herdr_threads::daemon::health::cooperative_receipt_line()
        )),
        "{value}"
    );
    assert!(
        notes.contains(&serde_json::json!(
            "harness codex not installed: no executable `codex` on the daemon's PATH"
        )),
        "{value}"
    );

    let up = run_with_path(&state, &host, &["doctor", "--debug"], &path);
    let report = text(&up.stdout);
    assert_eq!(up.status.code(), Some(0), "{report}{}", text(&up.stderr));
    assert!(report.contains("daemon: degraded"), "{report}");
    assert!(report.contains("daemon.version_matches: true"), "{report}");
    assert!(
        report.contains("daemon.harness_claude: cooperative"),
        "{report}"
    );
    assert!(report.contains("daemon.limitation: host "), "{report}");
    assert!(
        report.contains(&format!(
            "daemon.note: {}\n",
            herdr_threads::daemon::health::cooperative_receipt_line()
        )),
        "{report}"
    );
    assert!(
        report.contains("hooks.claude.setup_installed: yes"),
        "{report}"
    );
    assert!(
        report.contains(&format!(
            "hooks.claude.observed: {}\n",
            herdr_threads::cli::doctor::CLAUDE_NOT_OBSERVABLE
        )),
        "{report}"
    );
    assert!(!report.contains("\nlimitation: "), "{report}");
    assert!(report.contains("result: degraded"), "{report}");

    herdr_threads::harness::setup::remove_user_settings(kind, &settings, &manifest).unwrap();
    let removed = run(&state, &host, &["doctor", "--debug"], None);
    assert!(text(&removed.stdout).contains("hooks.claude.setup_installed: no"));
}

/// A scratch `PATH` directory holding a synthetic `codex`: a shell script
/// printing `version` whose unexecuted tail embeds `tail`.
fn codex_on_path(root: &Path, label: &str, version: &str, tail: &[u8]) -> PathBuf {
    let dir = root.join(format!("bin-{label}"));
    fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let mut bytes = format!("#!/bin/sh\nprintf 'codex-cli {version}\\n'\nexit 0\n").into_bytes();
    bytes.extend_from_slice(tail);
    let codex = dir.join("codex");
    fs::write(&codex, bytes).unwrap();
    fs::set_permissions(&codex, fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// The committed Codex 0.158.0 hook schema extraction, concatenated.
fn committed_codex_schemas() -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/evidence/codex-158-hook-capture/schemas-0.158.0");
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".schema.json"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for file in files {
        out.extend_from_slice(&fs::read(file).unwrap());
        out.push(b'\n');
    }
    out
}

/// Doctor reports the installed codex it would resolve on PATH: a listed
/// version as listed, an unlisted version whose embedded hook schemas match
/// as "schema-matched, live-unverified", and an unmatched one as refused
/// with the actionable message. None of them changes doctor's exit status.
/// Kills: doctor omitting the codex admission, reporting a schema-matched
/// version as listed, and hiding a schema refusal.
#[test]
fn doctor_reports_codex_schema_matched_admission() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let schemas = committed_codex_schemas();
    let mut changed = schemas.clone();
    let at = changed
        .windows(b"\"fork\"".len())
        .position(|w| w == b"\"fork\"")
        .unwrap();
    changed.splice(at..at + 6, b"\"forked\"".iter().copied());
    for (label, version, tail, admission, evidence) in [
        (
            "listed",
            "0.158.0",
            &b""[..],
            "listed",
            "codex 0.158.0: listed recipe codex-hooks-v1",
        ),
        (
            "matched",
            "0.160.0",
            &schemas[..],
            "schema-matched, live-unverified",
            "codex 0.160.0: schema-matched, live-unverified: recipe codex-hooks-v1 hook schemas \
             sha256:86858f2456c999030224a92d8dfb535183fe0edf8601690d8941978fadbb066d; binary sha256 ",
        ),
        (
            "unmatched",
            "0.160.0",
            &changed[..],
            "optimistic",
            "codex 0.160.0: optimistic (newer-than-verified): assumed recipe codex-hooks-v1; \
             schema drift sha256:",
        ),
    ] {
        let bin = codex_on_path(&scratch.0, label, version, tail);
        let output = scrubbed_command(BIN)
            .arg("--state-dir")
            .arg(&state)
            .arg("--host-endpoint")
            .arg(&host)
            .args(["--json", "doctor"])
            .env("HOME", scratch.0.join("home"))
            .env("CLAUDE_CONFIG_DIR", scratch.0.join("claude-config"))
            .env("CODEX_HOME", scratch.0.join("codex-home"))
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .current_dir(&scratch.0)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(3),
            "{label}: {}",
            text(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let installed = &value["doctor"]["hooks"]["codex"]["installed"];
        assert_eq!(installed["admission"], admission, "{label}: {installed}");
        assert_eq!(installed["binary"], bin.join("codex").display().to_string());
        let line = installed["evidence"].as_str().unwrap();
        assert!(line.starts_with(evidence), "{label}: {line}");
        // Schema-matched and optimistic admissions both name their recipe
        // and carry no error: the drifted binary is admitted, not refused.
        assert!(installed["error"].is_null(), "{label}: {installed}");
        assert_eq!(installed["recipe"], "codex-hooks-v1");
        assert_eq!(
            value["doctor"]["hooks"]["codex"]["socket_policy_validation"],
            "not_run"
        );
        if version == "0.160.0" {
            assert!(
                value["doctor"]["limitations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| {
                        row.as_str()
                            .is_some_and(|line| line.contains("target executable/effective config"))
                    }),
                "{value}"
            );
        }
    }
    let bin = codex_on_path(&scratch.0, "text", "0.160.0", &schemas);
    let output = scrubbed_command(BIN)
        .arg("--state-dir")
        .arg(&state)
        .arg("--host-endpoint")
        .arg(&host)
        .args(["doctor", "--debug"])
        .env("HOME", scratch.0.join("home"))
        .env("CLAUDE_CONFIG_DIR", scratch.0.join("claude-config"))
        .env("CODEX_HOME", scratch.0.join("codex-home"))
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_BIN_PATH")
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    let report = text(&output.stdout);
    assert!(
        report.contains(
            "hooks.codex.installed: codex 0.160.0: schema-matched, live-unverified: recipe \
             codex-hooks-v1"
        ),
        "{report}"
    );
}

fn assert_daemon_unavailable(label: &str, output: &Output) {
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "{label}: {stderr}");
    assert!(stderr.contains("(host_unavailable)"), "{label}: {stderr}");
    assert!(stderr.contains("daemon ensure"), "{label}: {stderr}");
    assert!(!stderr.contains("os error"), "{label}: {stderr}");
    assert!(
        output.stdout.is_empty(),
        "{label}: {}",
        text(&output.stdout)
    );
}

/// After `daemon stop` unlinks the endpoint descriptor (the namespace stays),
/// every CLI path that reads the descriptor reports the documented exit-3
/// daemon-unavailable result instead of a raw Io NotFound with exit 1.
/// Kills: reverting any one of the cooperative-selection, `seat resolve` or
/// `retry` descriptor reads in `cli::run` to a bare `read_descriptor(...)?`
/// (each site is exercised separately below), and dropping the NotFound
/// mapping from the shared published-endpoint reader.
#[test]
fn stopped_daemon_is_unavailable_for_every_descriptor_read() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };
    let ensure = run(&state, &host, &["daemon", "ensure"], None);
    assert_eq!(ensure.status.code(), Some(0), "{}", text(&ensure.stderr));
    let stop = run(&state, &host, &["daemon", "stop"], None);
    assert_eq!(stop.status.code(), Some(0), "{}", text(&stop.stderr));
    let instance_dir = fs::read_dir(state.join("instances"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(
        instance_dir.join("namespace").exists(),
        "stop keeps the namespace, so only the descriptor read can fail"
    );

    let cooperative = run(
        &state,
        &host,
        &[
            "--cooperative-seat",
            "s",
            "--cooperative-target",
            "w1:p1",
            "--cooperative-harness",
            "claude",
            "--cooperative-role",
            "top-level",
            "check-in",
        ],
        None,
    );
    assert_daemon_unavailable("cooperative check-in", &cooperative);
    let resolve = run(&state, &host, &["seat", "resolve", "--pane", "w1:p1"], None);
    assert_daemon_unavailable("seat resolve", &resolve);
    let retry = run(&state, &host, &["retry", "local:1"], None);
    assert_daemon_unavailable("retry", &retry);
    for args in [
        &["daemon", "health"][..],
        &["daemon", "stop"][..],
        &["view", "--once"][..],
    ] {
        assert_daemon_unavailable(&args.join(" "), &run(&state, &host, args, None));
    }
}

/// An unsafe (group-writable) state root is invalid local context: status 2
/// from both `daemon ensure` and `doctor`, matching the documented table.
/// Kills: `daemon ensure` wrapping the unsafe-root failure as
/// `HostUnavailable` (exit 3, "run daemon ensure", which cannot help), and
/// a doctor that stops classifying the unsafe root as status 2.
#[test]
fn unsafe_state_root_is_invalid_local_context_for_ensure_and_doctor() {
    let scratch = Scratch::new();
    let state = scratch.0.join("shared-state");
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o775)).unwrap();
    let host = scratch.0.join("host.sock");
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };
    let ensure = run(&state, &host, &["daemon", "ensure"], None);
    let stderr = text(&ensure.stderr);
    assert_eq!(ensure.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("unsafe state directory"), "{stderr}");
    assert!(stderr.contains("(invalid_request)"), "{stderr}");
    assert!(!state.join("instances").exists());

    let doctor = run(&state, &host, &["doctor"], None);
    assert_eq!(doctor.status.code(), Some(2), "{}", text(&doctor.stdout));
    assert!(text(&doctor.stdout).starts_with("doctor: unsafe_state_dir\n"));
    assert!(!state.join("instances").exists());
}

/// Bare invocation prints usage on stderr, not an error line with the help
/// text glued to `(invalid_request)`, and keeps exit status 2.
/// Kills: routing clap's DisplayHelpOnMissingArgumentOrSubcommand through
/// the `herdr-threads: DETAIL (code)` error renderer, and exiting 0 for it.
#[test]
fn bare_invocation_prints_usage_with_status_two() {
    let scratch = Scratch::new();
    let output = scrubbed_command(BIN)
        .current_dir(&scratch.0)
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PANE_ID")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "{}", text(&output.stdout));
    let stderr = text(&output.stderr);
    assert!(!stderr.starts_with("herdr-threads: "), "{stderr}");
    assert!(!stderr.contains("(invalid_request)"), "{stderr}");
    assert!(stderr.contains("Usage:"), "{stderr}");
    assert!(stderr.contains("Exit status:"), "{stderr}");
    assert!(stderr.ends_with('\n'), "{stderr:?}");
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
}

/// An unsafe plugin-created private directory (`instances/` or the instance
/// directory changed to 0755 under a safe root) is invalid local context:
/// `daemon ensure` and `doctor` agree on status 2 and `unsafe_state_dir`,
/// and doctor does not tell the operator to run `daemon ensure`.
/// Kills: "doctor ignores private-directory safety" (doctor only checks the
/// root, falls through to `not_running`, exit 3 with the ensure hint), and
/// dropping the `UnsafeLocalState` marker from `ensure_private_dir` (ensure
/// then exits 3 `host_unavailable`), doctor checking only `instances/` and
/// not the instance directory, and doctor still printing the ensure hint.
#[test]
fn unsafe_private_dir_gives_same_status_from_ensure_and_doctor() {
    for level in ["instances", "instance"] {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
        let host = scratch.0.join("host.sock");
        let _guard = DaemonGuard {
            state: state.clone(),
            host: host.clone(),
        };
        let probe = run(&state, &host, &["--json", "doctor"], None);
        let report: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let instance_dir = PathBuf::from(report["doctor"]["instance_dir"].as_str().unwrap());
        let instances = state.join("instances");
        assert_eq!(instance_dir.parent().unwrap(), instances);
        fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&instance_dir)
            .unwrap();
        let unsafe_dir = if level == "instances" {
            &instances
        } else {
            &instance_dir
        };
        fs::set_permissions(unsafe_dir, fs::Permissions::from_mode(0o755)).unwrap();

        let ensure = run(&state, &host, &["daemon", "ensure"], None);
        let stderr = text(&ensure.stderr);
        assert_eq!(ensure.status.code(), Some(2), "{level}: {stderr}");
        assert!(
            stderr.contains("unsafe private directory"),
            "{level}: {stderr}"
        );
        assert!(stderr.contains("(invalid_request)"), "{level}: {stderr}");

        let doctor = run(&state, &host, &["doctor"], None);
        let stdout = text(&doctor.stdout);
        assert_eq!(doctor.status.code(), Some(2), "{level}: {stdout}");
        assert!(
            stdout.starts_with("doctor: unsafe_state_dir\n"),
            "{level}: {stdout}"
        );
        assert!(
            stdout.contains("unsafe private directory"),
            "{level}: {stdout}"
        );
        assert!(
            !stdout.contains("run `herdr-threads daemon ensure`"),
            "{level}: {stdout}"
        );
        assert_eq!(
            fs::metadata(unsafe_dir).unwrap().permissions().mode() & 0o777,
            0o755,
            "{level}: diagnostics do not repair the mode"
        );
    }
}

/// A "no caller located" outcome is invalid local context: one
/// human-readable line naming the remedy, `(invalid_request)`, exit 2 and no
/// stdout. Never 4 ("unsupported capability", which callers read as "give
/// up") and never 1 (a failed request).
fn assert_caller_not_located(label: &str, output: &Output, remedy: &str) {
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{label}: {stderr}");
    assert!(stderr.starts_with("herdr-threads: "), "{label}: {stderr}");
    assert!(stderr.contains("(invalid_request)"), "{label}: {stderr}");
    assert!(stderr.contains(remedy), "{label}: {stderr}");
    assert!(
        output.stdout.is_empty(),
        "{label}: {}",
        text(&output.stdout)
    );
}

/// No pane and no `--cooperative-*`/`--seat`: a seat-acting command and both
/// seat-defaulting reads are the same "no caller located" classification,
/// decided before any daemon is needed (none is running here).
/// Kills: `derive_selection`'s missing-pane arm reverted to `unsupported`
/// (the prior exit 4), and the seat-default read's missing-pane arm changed
/// to anything other than `invalid_request` (exit 2).
#[test]
fn caller_without_pane_is_invalid_request_for_acting_and_reads() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let acting = run(&state, &host, &["thread", "create", "--topic", "x"], None);
    assert_caller_not_located("no pane: thread create", &acting, "--cooperative-seat");
    let inbox = run(&state, &host, &["inbox"], None);
    assert_caller_not_located("no pane: inbox", &inbox, "--seat SEAT");
    let pending = run(&state, &host, &["pending-receipts"], None);
    assert_caller_not_located("no pane: pending-receipts", &pending, "--seat SEAT");
}

/// With the daemon up, a pane that maps to no seat (for a seat-acting
/// command and for `inbox`) and a mapped seat that has no lifecycle
/// check-in context (derived from the pane, and selected explicitly with
/// `--cooperative-*`) all exit 2 with `(invalid_request)`. The mapped seat
/// is written straight into the store: no Herdr host is involved.
/// Kills: `no_seat_for_pane` reverted to `mapping_error` (target_unresolved,
/// exit 1); `derive_selection`'s no-context arm reverted to `unsupported`
/// (exit 4); `run_cooperative`'s missing-context arm reverted to
/// `unsupported` (exit 4).
#[test]
fn unmapped_pane_and_missing_context_are_invalid_request() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };
    let ensure = run(&state, &host, &["daemon", "ensure"], None);
    assert_eq!(ensure.status.code(), Some(0), "{}", text(&ensure.stderr));
    let pane = "w1:p1";

    let unmapped = run_in_pane(
        &state,
        &host,
        Some(pane),
        &["thread", "create", "--topic", "x"],
        None,
    );
    assert_caller_not_located(
        "unmapped pane: thread create",
        &unmapped,
        "seat resolve --pane",
    );
    let unmapped_inbox = run_in_pane(&state, &host, Some(pane), &["inbox"], None);
    assert_caller_not_located("unmapped pane: inbox", &unmapped_inbox, "--seat SEAT");

    let database = fs::read_dir(state.join("instances"))
        .unwrap()
        .map(|entry| entry.unwrap().path().join("threads.sqlite3"))
        .find(|path| path.exists())
        .expect("instance database");
    let db = rusqlite::Connection::open(&database).unwrap();
    db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
    let instance: String = db
        .query_row("SELECT id FROM host_instances", [], |r| r.get(0))
        .unwrap();
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,target_id,generation,created_at) \
         VALUES('s-no-context',?1,'resolved','operator_fresh',?2,1,0)",
        [instance.as_str(), pane],
    )
    .unwrap();
    drop(db);

    let derived = run_in_pane(
        &state,
        &host,
        Some(pane),
        &["thread", "create", "--topic", "x"],
        None,
    );
    assert_caller_not_located(
        "mapped pane, no context",
        &derived,
        "no lifecycle check-in context",
    );
    let explicit = run(
        &state,
        &host,
        &[
            "--cooperative-seat",
            "s-no-context",
            "--cooperative-target",
            pane,
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
            "thread",
            "create",
            "--topic",
            "x",
        ],
        None,
    );
    assert_caller_not_located(
        "explicit selection, no context",
        &explicit,
        "lifecycle check-in",
    );
}

fn endpoint_json(state: &Path) -> serde_json::Value {
    let instance_dir = fs::read_dir(state.join("instances"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    serde_json::from_slice(&fs::read(instance_dir.join("endpoint.json")).unwrap()).unwrap()
}

/// ht-910 P1: `setup codex` writes the narrow workspace-write sandbox
/// allowance into the scratch CODEX_HOME's config.toml, naming exactly the
/// socket the daemon binds, and that pathname is the same after a daemon
/// restart (a new boot ID, the same allowlist). Kills: a boot-specific socket
/// name (the allowlist goes stale on restart), setup computing a different
/// instance than the daemon, dropping any of the three keys, and guessing a
/// socket when the Herdr instance is unknown.
#[test]
fn setup_codex_sandbox_allowance_names_the_stable_daemon_socket() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    // 0.159.2 (like 0.159.3) is a version whose proxy default-deny was measured; it
    // is admitted as schema-matched, so the fake embeds the recipe schemas.
    let bin = codex_on_path(&scratch.0, "codex", "0.159.2", &committed_codex_schemas());
    let codex = bin.join("codex").display().to_string();
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };
    // Codex config is written only to a scratch CODEX_HOME, never the user's.
    let codex_home = scratch.0.join("codex-home");
    let setup = || {
        let mut command = scrubbed_command(BIN);
        command
            .arg("--state-dir")
            .arg(&state)
            .arg("--host-endpoint")
            .arg(&host)
            .args(["--json", "setup", "codex", "--harness-binary", &codex])
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PLUGIN_STATE_DIR");
        scratch_homes(&mut command, &state);
        let out = command.output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["setup"].clone()
    };
    let planned = setup();
    assert!(
        !state.join("instances").exists(),
        "setup codex must not create daemon state"
    );
    assert_eq!(planned["action"], "installed", "{planned}");
    assert_eq!(planned["created_config_file"], true, "{planned}");

    let ensure = run(&state, &host, &["daemon", "ensure"], None);
    assert_eq!(ensure.status.code(), Some(0), "{}", text(&ensure.stderr));
    let first = endpoint_json(&state);
    let socket = first["endpoint"].as_str().unwrap().to_owned();
    assert!(socket.len() < 100, "{socket}");
    assert_eq!(planned["sandbox"]["socket_path"], socket);
    assert_eq!(planned["sandbox"]["present"], true);
    let config = fs::read_to_string(codex_home.join("config.toml")).unwrap();
    let doc: toml_edit::DocumentMut = config.parse().unwrap();
    assert_eq!(
        doc["sandbox_workspace_write"]["network_access"].as_bool(),
        Some(true)
    );
    assert_eq!(
        doc["features"]["network_proxy"]["enabled"].as_bool(),
        Some(true)
    );
    let sockets = doc["features"]["network_proxy"]["unix_sockets"]
        .as_table_like()
        .unwrap();
    assert_eq!(sockets.len(), 1, "{config}");
    assert_eq!(sockets.get(&socket).and_then(|v| v.as_str()), Some("allow"));
    let note = planned["sandbox"]["note"].as_str().unwrap();
    assert!(note.contains("network_access=true") && note.contains("stays"));
    assert!(note.contains("denied") && note.contains("state directory"));

    let stop = run(&state, &host, &["daemon", "stop"], None);
    assert_eq!(stop.status.code(), Some(0), "{}", text(&stop.stderr));
    let again = run(&state, &host, &["daemon", "ensure"], None);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again.stderr));
    let second = endpoint_json(&state);
    assert_eq!(
        second["endpoint"], first["endpoint"],
        "socket path changed on restart"
    );
    assert_ne!(second["boot_id"], first["boot_id"]);
    let rerun = setup();
    assert_eq!(rerun["action"], "already_installed", "{rerun}");
    assert_eq!(rerun["sandbox"], planned["sandbox"]);
    assert_eq!(
        fs::read_to_string(codex_home.join("config.toml")).unwrap(),
        config
    );

    // ht-4is.8.20: the writable roots are exactly the two client-side journal directories the
    // CLI writes, as the canonical paths it opens, never the instance directory itself.
    let instance = fs::read_dir(state.join("instances"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .canonicalize()
        .unwrap();
    let expected_roots = vec![
        instance.join("intents").display().to_string(),
        instance.join("contexts").display().to_string(),
    ];
    let roots: Vec<String> = doc["sandbox_workspace_write"]["writable_roots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(roots, expected_roots, "{config}");
    assert_eq!(
        planned["sandbox"]["writable_roots"],
        serde_json::json!(expected_roots)
    );
    assert_eq!(
        planned["sandbox"]["writable_roots_present"], true,
        "{planned}"
    );
    assert!(note.contains("writable_roots") && note.contains("SQLite database"));

    // An allowance recorded before the roots existed (manifest version 1): setup-status and
    // doctor say sandboxed mutations fail and name `setup codex`, which upgrades it in place.
    let config_path = codex_home.join("config.toml");
    let manifest = PathBuf::from(planned["config_manifest"].as_str().unwrap());
    assert!(herdr_threads::harness::codex_config::remove(&config_path, &manifest).unwrap());
    herdr_threads::harness::codex_config::install(&config_path, &manifest, &socket, &[]).unwrap();
    let status = |verb: &str| {
        let mut command = scrubbed_command(BIN);
        command
            .arg("--state-dir")
            .arg(&state)
            .arg("--host-endpoint")
            .arg(&host)
            .args(["--json", verb, "codex", "--harness-binary", &codex])
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PLUGIN_STATE_DIR");
        scratch_homes(&mut command, &state);
        let out = command.output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["setup"].clone()
    };
    let old = status("setup-status");
    assert_eq!(old["sandbox"]["present"], true, "{old}");
    assert_eq!(old["sandbox"]["writable_roots_present"], false, "{old}");
    let warnings = old["warnings"].to_string();
    assert!(
        warnings.contains("Operation not") && warnings.contains("herdr-threads setup codex"),
        "{old}"
    );
    let mut command = scrubbed_command(BIN);
    command
        .arg("--state-dir")
        .arg(&state)
        .arg("--host-endpoint")
        .arg(&host)
        .args(["--json", "doctor"])
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_PANE_ID")
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    scratch_homes(&mut command, &state);
    let doctor: serde_json::Value =
        serde_json::from_slice(&command.output().unwrap().stdout).unwrap();
    let roots_warning = &doctor["doctor"]["hooks"]["codex"]["sandbox_roots_warning"];
    assert!(
        roots_warning
            .as_str()
            .is_some_and(|w| w.contains("writable roots")),
        "{doctor}"
    );
    assert!(
        doctor["doctor"]["limitations"]
            .to_string()
            .contains("writable roots"),
        "{doctor}"
    );
    let upgraded = status("setup");
    assert_eq!(upgraded["action"], "installed", "{upgraded}");
    assert_eq!(
        upgraded["sandbox"]["writable_roots_present"], true,
        "{upgraded}"
    );
    let fixed = status("setup-status");
    assert!(fixed["warnings"].is_null(), "{fixed}");
    assert_eq!(fs::read_to_string(&config_path).unwrap(), config);

    // Without a known (or detectable) host endpoint nothing is guessed:
    // setup refuses (status 2) and writes nothing more.
    let mut command = scrubbed_command(BIN);
    command
        .arg("--state-dir")
        .arg(&state)
        .args(["--json", "setup", "codex", "--harness-binary", &codex])
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env("PATH", "/usr/bin:/bin");
    scratch_homes(&mut command, &state);
    let bare = command.output().unwrap();
    assert_eq!(bare.status.code(), Some(2), "{}", text(&bare.stdout));
    assert!(
        text(&bare.stderr).contains("Herdr host endpoint unknown"),
        "{}",
        text(&bare.stderr)
    );
}

/// ht-910 review: `network_access=true` is only narrow where Codex's proxy
/// enforces default-deny, which was measured on 0.159.2 and 0.159.3 alone. Any other
/// admitted version (a listed recipe version, or another schema-matched one)
/// gets the user-level hooks and a warning, never the allowance, in both
/// install and status. Kills: writing the allowance for every admitted
/// version.
#[test]
fn setup_codex_withholds_sandbox_allowance_on_unmeasured_versions() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let codex_home = scratch.0.join("codex-home");
    let schemas = committed_codex_schemas();
    for (label, version, tail) in [
        ("listed", "0.158.0", &b""[..]),
        ("listed-old", "0.157.1", &b""[..]),
        ("matched", "0.160.0", &schemas[..]),
    ] {
        let bin = codex_on_path(&scratch.0, label, version, tail);
        let codex = bin.join("codex").display().to_string();
        let setup = |verb: &str| {
            let mut command = scrubbed_command(BIN);
            command
                .arg("--state-dir")
                .arg(&state)
                .arg("--host-endpoint")
                .arg(&host)
                .args(["--json", verb, "codex", "--harness-binary", &codex])
                .env_remove("HERDR_SOCKET_PATH")
                .env_remove("HERDR_PLUGIN_STATE_DIR");
            scratch_homes(&mut command, &state);
            let out = command.output().unwrap();
            assert_eq!(out.status.code(), Some(0), "{label}: {}", text(&out.stderr));
            serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["setup"].clone()
        };
        let planned = setup("setup");
        let sandbox = &planned["sandbox"];
        assert!(sandbox["socket_path"].is_null(), "{label}: {sandbox}");
        let omitted = sandbox["omitted"].as_str().unwrap();
        assert!(
            omitted.contains(&format!("unmeasured on Codex {version}"))
                && omitted.contains("0.159.2"),
            "{label}: {omitted}"
        );
        assert!(
            omitted.contains("controlled denied sockets"),
            "{label}: {omitted}"
        );
        assert!(!omitted.contains("-- curl"), "{label}: {omitted}");
        assert!(
            planned["warnings"]
                .to_string()
                .contains("sandbox socket allowance not written"),
            "{label}: {planned}"
        );
        assert!(
            codex_home.join("hooks.json").exists(),
            "{label}: hooks are still installed"
        );
        assert!(
            !codex_home.join("config.toml").exists(),
            "{label}: no allowance written"
        );
        let status = setup("setup-status");
        assert_eq!(status["installed"], true, "{label}: {status}");
        assert!(
            status["sandbox"]["socket_path"].is_null(),
            "{label}: {status}"
        );
    }
    assert!(
        !state.join("instances").exists(),
        "setup codex must not create daemon state"
    );
}

/// ht-4is.8.8 review: the allowance lives in config.toml and applies to
/// every Codex session of that CODEX_HOME, so the version gate must keep
/// being checked after install. With an allowance recorded on 0.159.2 and
/// Codex then upgraded to an unmeasured version, re-running setup, status
/// and doctor all warn loudly that `network_access=true` is still installed
/// and name `unsetup codex`; setup no longer claims the allowance was "not
/// written". On the measured version nothing warns, and after unsetup the
/// warning is gone. Kills: checking the gate only when writing, and the
/// misleading "not written" warning while the old allowance is live.
#[test]
fn codex_allowance_on_an_unmeasured_version_warns_in_setup_status_and_doctor() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let codex_home = scratch.0.join("codex-home");
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };
    let schemas = committed_codex_schemas();
    let measured = codex_on_path(&scratch.0, "measured", "0.159.2", &schemas);
    let upgraded = codex_on_path(&scratch.0, "upgraded", "0.160.0", &schemas);
    let invoke = |bin: &Path, args: &[&str]| {
        let mut command = scrubbed_command(BIN);
        command
            .arg("--state-dir")
            .arg(&state)
            .arg("--host-endpoint")
            .arg(&host)
            .args(args)
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()));
        scratch_homes(&mut command, &state);
        command.output().unwrap()
    };
    let setup = |bin: &Path, verb: &str| {
        let codex = bin.join("codex").display().to_string();
        let out = invoke(bin, &["--json", verb, "codex", "--harness-binary", &codex]);
        assert_eq!(out.status.code(), Some(0), "{verb}: {}", text(&out.stderr));
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["setup"].clone()
    };
    let doctor = |bin: &Path| {
        let out = invoke(bin, &["--json", "doctor"]);
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["doctor"].clone()
    };
    let loud = |value: &str| {
        value.contains("WARNING")
            && value.contains("network_access=true is installed")
            && value.contains("unmeasured Codex version")
            && value.contains("herdr-threads unsetup codex")
    };

    let installed = setup(&measured, "setup");
    assert_eq!(installed["sandbox"]["present"], true, "{installed}");
    assert!(installed["sandbox"]["warning"].is_null(), "{installed}");
    let status = setup(&measured, "setup-status");
    assert!(status["sandbox"]["warning"].is_null(), "{status}");
    assert!(status["warnings"].is_null(), "{status}");
    let measured_doctor = doctor(&measured);
    assert!(
        measured_doctor["hooks"]["codex"]["sandbox_warning"].is_null(),
        "{measured_doctor}"
    );
    let config = fs::read_to_string(codex_home.join("config.toml")).unwrap();

    // ht-4is.8.15: Codex auto-updated 0.159.2 -> 0.159.3, whose default-deny
    // was measured too (codex-1593-sandbox-probe): the recorded allowance is
    // still admitted, so setup, status and doctor stay quiet and nothing is
    // rewritten.
    let patch = codex_on_path(&scratch.0, "patch", "0.159.3", &schemas);
    let rerun = setup(&patch, "setup");
    assert_eq!(rerun["action"], "already_installed", "{rerun}");
    assert_eq!(rerun["sandbox"]["present"], true, "{rerun}");
    assert!(rerun["sandbox"]["warning"].is_null(), "{rerun}");
    assert!(
        rerun["warnings"].as_array().is_some_and(Vec::is_empty),
        "{rerun}"
    );
    let status = setup(&patch, "setup-status");
    assert!(status["sandbox"]["warning"].is_null(), "{status}");
    assert!(status["warnings"].is_null(), "{status}");
    let patch_doctor = doctor(&patch);
    assert!(
        patch_doctor["hooks"]["codex"]["sandbox_warning"].is_null(),
        "{patch_doctor}"
    );
    assert_eq!(
        fs::read_to_string(codex_home.join("config.toml")).unwrap(),
        config
    );

    // Codex upgraded: the allowance from the measured version stays.
    let rerun = setup(&upgraded, "setup");
    let warnings = rerun["warnings"].to_string();
    assert!(
        loud(&warnings) && warnings.contains("Codex 0.160.0"),
        "{rerun}"
    );
    assert!(
        !warnings.contains("sandbox socket allowance not written"),
        "{rerun}"
    );
    assert_eq!(rerun["sandbox"]["unmeasured_installed"], true, "{rerun}");
    assert!(
        loud(rerun["sandbox"]["warning"].as_str().unwrap()),
        "{rerun}"
    );
    assert_eq!(
        fs::read_to_string(codex_home.join("config.toml")).unwrap(),
        config,
        "setup must not touch the allowance on an unmeasured version"
    );
    let status = setup(&upgraded, "setup-status");
    assert_eq!(status["sandbox"]["unmeasured_installed"], true, "{status}");
    assert!(loud(&status["warnings"].to_string()), "{status}");
    let codex = upgraded.join("codex").display().to_string();
    let text_status = invoke(
        &upgraded,
        &["setup-status", "codex", "--harness-binary", &codex],
    );
    assert!(
        text(&text_status.stdout).contains("warning: WARNING"),
        "{}",
        text(&text_status.stdout)
    );
    let upgraded_doctor = doctor(&upgraded);
    assert!(
        upgraded_doctor["hooks"]["codex"]["sandbox_warning"]
            .as_str()
            .is_some_and(loud),
        "{upgraded_doctor}"
    );
    let doctor_text = invoke(&upgraded, &["doctor", "--debug"]);
    assert!(
        text(&doctor_text.stdout).contains("hooks.codex.sandbox_warning: WARNING"),
        "{}",
        text(&doctor_text.stdout)
    );

    // After unsetup nothing is installed and nothing warns.
    let out = invoke(&upgraded, &["--json", "unsetup", "codex"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let removed =
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["setup"].clone();
    assert_eq!(removed["action"], "removed", "{removed}");
    let status = setup(&upgraded, "setup-status");
    assert!(status["sandbox"]["warning"].is_null(), "{status}");
    assert!(status["warnings"].is_null(), "{status}");
    let after = doctor(&upgraded);
    assert!(
        after["hooks"]["codex"]["sandbox_warning"].is_null(),
        "{after}"
    );
}

/// The instance paths a `--state-dir`/`--host-endpoint` pair resolves to, with
/// the instance directory prepared (so a test can plant files in it).
fn prepared_instance(state: &Path, host: &Path) -> herdr_threads::daemon::paths::InstancePaths {
    let context = herdr_threads::daemon::paths::RuntimeContext::explicit(
        state.to_path_buf(),
        host.to_path_buf(),
        None,
    )
    .unwrap();
    let paths = herdr_threads::daemon::paths::InstancePaths::resolve(&context).unwrap();
    paths.prepare_instance_dir().unwrap();
    paths
}

/// ht-p03.11. Kills: a detached child whose pre-election failure goes to a
/// null stderr (the operator sees only a timeout), and a tail that does not
/// name the attempt's own startup file.
#[test]
fn ensure_prints_the_startup_log_tail_when_the_daemon_fails_before_election() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let paths = prepared_instance(&state, &host);
    fs::write(
        &paths.database_path,
        b"this is not a sqlite database at all",
    )
    .unwrap();

    let ensure = run(&state, &host, &["daemon", "ensure"], None);
    let stderr = text(&ensure.stderr);
    assert_eq!(ensure.status.code(), Some(3), "{stderr}");
    let logs = paths.instance_dir.join("logs");
    let files: Vec<_> = fs::read_dir(&logs).unwrap().flatten().collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let file = files[0].path();
    assert!(stderr.contains(&file.display().to_string()), "{stderr}");
    assert!(stderr.contains("store startup"), "{stderr}");
    assert!(
        fs::read_to_string(&file).unwrap().contains("store startup"),
        "the child's error is in its own attempt file"
    );
}

/// ht-p03.11. Kills: no fallback when `<instance>/logs` cannot be created
/// (the child's error is lost), and a fallback with a different exit status.
#[test]
fn ensure_falls_back_to_piped_stderr_when_the_logs_dir_is_unusable() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let paths = prepared_instance(&state, &host);
    fs::write(
        &paths.database_path,
        b"this is not a sqlite database at all",
    )
    .unwrap();
    fs::write(paths.instance_dir.join("logs"), b"in the way").unwrap();

    let ensure = run(&state, &host, &["daemon", "ensure"], None);
    let stderr = text(&ensure.stderr);
    assert_eq!(ensure.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("startup log unavailable"), "{stderr}");
    assert!(stderr.contains("showing the child's stderr"), "{stderr}");
    assert!(stderr.contains("store startup"), "{stderr}");
}

/// ht-p03.11 (fallback survival). Kills: a daemon started with a piped stderr
/// that keeps writing to the pipe after its starter exits (EPIPE panic or
/// death): after `ensure` returns the daemon must still answer Health, and
/// its later stderr lines must reach daemon.log.
#[test]
fn fallback_daemon_survives_starter_exit() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let paths = prepared_instance(&state, &host);
    fs::write(paths.instance_dir.join("logs"), b"in the way").unwrap();
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };

    let ensure = run(&state, &host, &["daemon", "ensure"], None);
    assert_eq!(ensure.status.code(), Some(0), "{}", text(&ensure.stderr));
    // The starter has exited. The host endpoint does not exist, so the
    // observation lane fails and logs through the daemon's stderr.
    let log = paths.instance_dir.join("daemon.log");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let logged = fs::read_to_string(&log).unwrap_or_default();
        if logged.contains("lane observation:") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no lane line in daemon.log: {logged}"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let health = run(&state, &host, &["daemon", "health"], None);
    assert_eq!(health.status.code(), Some(0), "{}", text(&health.stderr));
}

/// Pids of `daemon run` processes serving `state`.
fn daemon_pids(state: &Path) -> Vec<u32> {
    let needle = format!("daemon run --state-dir {} ", state.display());
    let ps = Command::new("/bin/ps")
        .args(["-axo", "pid=,command="])
        .output()
        .unwrap();
    text(&ps.stdout)
        .lines()
        .filter(|line| line.contains(&needle))
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect()
}

/// Waits up to `limit` for no daemon to serve `state`.
fn daemons_gone_within(state: &Path, limit: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + limit;
    loop {
        if daemon_pids(state).is_empty() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// A daemon started for a test exits once its owning test process dies, even
/// when the owner is SIGKILLed and no guard or Drop runs (ht-6y1). The owner
/// is a stand-in process named by the test-owner variable, killed abruptly.
/// Kills: dropping the owner watch from `daemon run`, or `daemon ensure`'s
/// detached spawn not inheriting the variable.
#[cfg(feature = "test-support")]
#[test]
fn daemon_exits_when_its_killed_test_owner_is_gone() {
    use herdr_threads::daemon::lifecycle::TEST_OWNER_PID_ENV as OWNER_PID_ENV;
    use std::time::Duration;
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    // Backstop only: the assertion below is what proves the owner watch.
    let _guard = DaemonGuard {
        state: state.clone(),
        host: host.clone(),
    };
    let mut owner = Command::new("/bin/sleep").arg("600").spawn_owned().unwrap();
    // leak-guard: sets a fake owner pid on purpose (the daemon must exit when that owner dies)
    let mut command = Command::new(BIN);
    command
        .arg("--state-dir")
        .arg(&state)
        .arg("--host-endpoint")
        .arg(&host)
        .args(["daemon", "ensure"])
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_BIN_PATH")
        .env(OWNER_PID_ENV, owner.id().to_string());
    scratch_homes(&mut command, &state);
    let ensure = command.output().unwrap();
    assert_eq!(ensure.status.code(), Some(0), "{}", text(&ensure.stderr));
    assert_eq!(daemon_pids(&state).len(), 1, "one detached daemon runs");
    // While the owner lives, the daemon keeps running.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(daemon_pids(&state).len(), 1, "daemon outlives a live owner");

    owner.kill().unwrap();
    owner.wait().unwrap();
    assert!(
        daemons_gone_within(&state, Duration::from_secs(5)),
        "daemon {:?} outlived its killed owner",
        daemon_pids(&state)
    );
}

/// A panic inside a test still stops the daemon it started: the guard's Drop
/// runs on unwind. Kills: a guard that does not stop the daemon on drop.
#[test]
fn daemon_guard_stops_the_daemon_on_panic_unwind() {
    let scratch = Scratch::new();
    let state = scratch.0.join("state");
    let host = scratch.0.join("host.sock");
    let (state_in, host_in) = (state.clone(), host.clone());
    let unwound = std::panic::catch_unwind(move || {
        let _guard = DaemonGuard {
            state: state_in.clone(),
            host: host_in.clone(),
        };
        let ensure = run(&state_in, &host_in, &["daemon", "ensure"], None);
        assert_eq!(ensure.status.code(), Some(0), "{}", text(&ensure.stderr));
        assert_eq!(daemon_pids(&state_in).len(), 1);
        panic!("simulated test failure");
    });
    assert!(unwound.is_err());
    assert!(
        daemons_gone_within(&state, std::time::Duration::from_secs(5)),
        "daemon {:?} outlived the panicking test",
        daemon_pids(&state)
    );
}
