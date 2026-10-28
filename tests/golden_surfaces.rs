//! Golden snapshots of the per-harness surfaces (ht-poh.2.7): byte-identity
//! guard for the core sweep and the delivery re-fit. Every surface is produced
//! by the built binary against an isolated scratch root (HOME, CLAUDE_CONFIG_DIR,
//! CODEX_HOME, state dir and a private PATH of fake harnesses), normalized by
//! [`Norm`] and compared byte for byte with `tests/fixtures/golden/**`.
//!
//! `HT_BLESS=1 cargo test --all-features --test combined golden_surfaces::`
//! rewrites the fixtures (the `tests/cli/golden_contract.rs` precedent). A
//! leaf that is allowed to change a surface (core-sweep spec D3: help text)
//! edits the fixture explicitly in its own commit.
//!
//! Layout: `<surface>/<harness>.<form>`; each fixture holds the normalized
//! stdout and a sibling `<fixture>.status` holds the exit status and stderr.
//! Normalization rewrites only run-specific bytes: the scratch roots, the
//! binary path, the package version, RFC 3339 timestamps and millisecond
//! epochs, UUIDs, 32/64-digit hashes derived from those paths, and the
//! interpreter path of the synthetic Hermes launcher. Everything harness-named
//! stays pinned.

use herdr_threads::{
    app::SystemClock,
    daemon::{
        ownership::OwnerLock,
        paths::{NodePaths, RuntimeContext},
    },
    harness::context::{ContextJournal, Harness, OccupantContext, Role, SessionReference},
    host::native::NativeCli,
    ports::{BootstrapObserver, DurableWorkAdmission, HostCallContext, SnapshotHeader, StorePort},
    protocol::time::{CallBudget, Cancellation, Clock, MonoInstant},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::{
        isolation::TestIsolation,
        spawn::{self, OwnedChild, SpawnOwned},
    },
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

// ---------------------------------------------------------------------------
// Comparator and normalizer
// ---------------------------------------------------------------------------

fn golden_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden")
}

/// Compare `actual` with the fixture `name` byte for byte, or rewrite the
/// fixture under `HT_BLESS=1`.
fn golden(name: &str, actual: &str) {
    let path = golden_root().join(name);
    if std::env::var_os("HT_BLESS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "golden fixture {} missing ({e}); bless with HT_BLESS=1",
            path.display()
        )
    });
    if expected == actual {
        return;
    }
    let (mut want, mut got) = (expected.lines(), actual.lines());
    let mut line = 1;
    loop {
        match (want.next(), got.next()) {
            (Some(a), Some(b)) if a == b => line += 1,
            (a, b) => panic!(
                "golden fixture {} differs at line {line}:\n  expected: {}\n  actual:   {}\n(HT_BLESS=1 rewrites it)",
                path.display(),
                a.unwrap_or("<end of file>"),
                b.unwrap_or("<end of file>"),
            ),
        }
    }
}

/// Run-specific bytes to rewrite before a comparison.
struct Norm {
    literals: Vec<(String, &'static str)>,
}
impl Norm {
    /// `roots` are directories (also matched in canonical form).
    fn new(roots: &[&Path]) -> Self {
        let mut literals: Vec<(String, &'static str)> = Vec::new();
        for root in roots {
            literals.push((root.display().to_string(), "<root>"));
            if let Ok(canonical) = root.canonicalize() {
                literals.push((canonical.display().to_string(), "<root>"));
            }
        }
        literals.push((BIN.to_owned(), "<bin>"));
        if let Ok(canonical) = Path::new(BIN).canonicalize() {
            literals.push((canonical.display().to_string(), "<bin>"));
        }
        // Longest first, so a root nested in another is not half-rewritten.
        literals.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        literals.dedup();
        Self { literals }
    }
    fn also(mut self, from: impl Into<String>, to: &'static str) -> Self {
        self.literals.push((from.into(), to));
        self.literals
            .sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        self
    }
    fn apply(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for (from, to) in &self.literals {
            if !from.is_empty() {
                text = text.replace(from.as_str(), to);
            }
        }
        scan(&replace_version(&text))
    }
}

/// `CARGO_PKG_VERSION` as a whole version number only (not inside `10.5.0`).
fn replace_version(text: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(version) {
        let before = rest[..at].chars().next_back();
        let after = rest[at + version.len()..].chars().next();
        let glued = |c: Option<char>| c.is_some_and(|c| c.is_ascii_digit() || c == '.');
        out.push_str(&rest[..at]);
        out.push_str(if glued(before) || glued(after) {
            version
        } else {
            "<version>"
        });
        rest = &rest[at + version.len()..];
    }
    out.push_str(rest);
    out
}

/// Timestamps, millisecond epochs, UUIDs and path-derived hashes.
fn scan(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let alnum = |at: Option<&u8>| at.is_some_and(u8::is_ascii_alphanumeric);
    while i < b.len() {
        let bounded_before = i == 0 || !alnum(b.get(i - 1));
        if bounded_before && let Some((len, tag)) = token_at(b, i) {
            out.push_str(tag);
            i += len;
            continue;
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn token_at(b: &[u8], i: usize) -> Option<(usize, &'static str)> {
    let digit = |at: usize| b.get(at).is_some_and(u8::is_ascii_digit);
    let hex = |at: usize| {
        b.get(at)
            .is_some_and(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    };
    let alnum = |at: usize| b.get(at).is_some_and(u8::is_ascii_alphanumeric);
    // 2026-10-01T09:30:00(.123)(Z|+01:00)
    let shape = b"dddd-dd-ddTdd:dd:dd";
    if shape.iter().enumerate().all(|(k, s)| match s {
        b'd' => digit(i + k),
        other => b.get(i + k) == Some(other),
    }) {
        let mut end = i + shape.len();
        if b.get(end) == Some(&b'.') && digit(end + 1) {
            end += 1;
            while digit(end) {
                end += 1;
            }
        }
        if b.get(end) == Some(&b'Z') {
            end += 1;
        } else if matches!(b.get(end), Some(b'+' | b'-')) && digit(end + 1) && digit(end + 2) {
            end += 3;
            if b.get(end) == Some(&b':') && digit(end + 1) && digit(end + 2) {
                end += 3;
            }
        }
        return Some((end - i, "<time>"));
    }
    // UUID 8-4-4-4-12
    let uuid = b"hhhhhhhh-hhhh-hhhh-hhhh-hhhhhhhhhhhh";
    if uuid.iter().enumerate().all(|(k, s)| match s {
        b'h' => hex(i + k),
        other => b.get(i + k) == Some(other),
    }) && !alnum(i + uuid.len())
    {
        return Some((uuid.len(), "<uuid>"));
    }
    // A maximal hex run of exactly 64 or 32 digits (path-derived hashes).
    let mut end = i;
    while hex(end) {
        end += 1;
    }
    if !alnum(end) {
        match end - i {
            64 => return Some((64, "<hash64>")),
            32 => return Some((32, "<hash32>")),
            _ => {}
        }
    }
    // A 13-digit millisecond epoch.
    let mut end = i;
    while digit(end) {
        end += 1;
    }
    if end - i == 13 && b[i] == b'1' && !alnum(end) {
        return Some((13, "<time>"));
    }
    None
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Record one command's result: `name` holds the normalized stdout, and
/// `name.status` the exit status and the normalized stderr.
fn record(norm: &Norm, name: &str, out: &Output) {
    golden(name, &norm.apply(&text(&out.stdout)));
    golden(
        &format!("{name}.status"),
        &format!(
            "exit: {}\nstderr:\n{}",
            out.status
                .code()
                .map_or_else(|| "signal".to_owned(), |c| c.to_string()),
            norm.apply(&text(&out.stderr))
        ),
    );
}

fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap();
    text.push('\n');
    text
}

// ---------------------------------------------------------------------------
// Scratch root (setup/doctor/help/hook surfaces)
// ---------------------------------------------------------------------------

/// A spawned binary with every inherited HERDR_/CLAUDE/CODEX variable removed;
/// a test sets the variables it needs after this call.
fn scrubbed_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    spawn::command(program)
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
    /// A private root. `short` roots live under the short socket base (a
    /// daemon's sockets sit below the state directory).
    fn new(short: bool) -> Self {
        let base = if short {
            PathBuf::from(herdr_threads::test_support::SHORT_TMP)
        } else {
            std::env::temp_dir()
        };
        let root = base.join(format!(
            "htgold-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let root = root.canonicalize().unwrap();
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

    fn norm(&self) -> Norm {
        Norm::new(&[&self.root])
    }

    /// A fake harness whose `--version` prints exactly `line`.
    fn harness(&self, name: &str, line: &str) {
        let path = self.bin.join(name);
        fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{line}'\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Fake `claude` and `codex` at the versions the fixtures were taken with.
    fn harnesses(&self) {
        self.harness("claude", "2.1.286 (Claude Code)");
        self.harness("codex", "codex-cli 0.158.0");
    }

    fn host(&self) -> PathBuf {
        self.root.join("host.sock")
    }

    /// The scratch environment: no Herdr instance on the command line or in
    /// the environment, no inherited markers.
    fn command(&self) -> Command {
        self.configure_command(scrubbed_command(BIN))
    }

    fn configure_command(&self, mut command: Command) -> Command {
        command
            .current_dir(&self.root)
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .env("HOME", &self.home)
            .env("CLAUDE_CONFIG_DIR", &self.claude_config)
            .env("CODEX_HOME", &self.codex_home)
            .env(
                herdr_threads::harness::claude_mod::TEST_MANAGED_SETTINGS_ENV,
                self.root.join("no-managed-settings.json"),
            )
            // The Codex pane environment's startup files follow $SHELL: pin it.
            .env("SHELL", "/bin/zsh")
            .env_remove("ZDOTDIR")
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("HERDR_SESSION")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_ENV");
        command
    }

    /// The instance flags, then `args`. An escalating command's leading words (`setup`,
    /// `unsetup`, `doctor fix`) come first: the CLI refuses options before them.
    fn instance_args<'a>(&self, command: &'a mut Command, args: &[&str]) -> &'a mut Command {
        let at = args
            .iter()
            .position(|arg| !arg.starts_with('-'))
            .unwrap_or(args.len());
        let words = match &args[at..] {
            ["setup" | "unsetup", ..] => 1,
            ["doctor", "fix", ..] => 2,
            _ => 0,
        };
        command
            .args(&args[at..at + words])
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(self.host())
            .args(&args[..at])
            .args(&args[at + words..])
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = self.command();
        self.instance_args(&mut command, args).output().unwrap()
    }

    /// `run` that must exit 0.
    fn ok(&self, args: &[&str]) -> Output {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            text(&out.stdout),
            text(&out.stderr)
        );
        out
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Stops the daemon of a scratch instance, even when an assertion fails.
struct DaemonGuard<'a>(&'a Scratch);
impl<'a> DaemonGuard<'a> {
    /// Starts the daemon and waits until it answers `daemon health`.
    fn start(scratch: &'a Scratch) -> Self {
        let guard = Self(scratch);
        scratch.ok(&["daemon", "ensure"]);
        let deadline = Instant::now() + Duration::from_secs(30);
        while !scratch.run(&["daemon", "health"]).status.success() {
            assert!(Instant::now() < deadline, "daemon never answered health");
            std::thread::sleep(Duration::from_millis(50));
        }
        guard
    }
}
impl Drop for DaemonGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.run(&["daemon", "stop"]);
    }
}

// ---------------------------------------------------------------------------
// Hermes (synthetic launcher, as tests/setup_cli.rs::HermesScopeFixture)
// ---------------------------------------------------------------------------

struct HermesScope {
    scratch: Scratch,
    launcher: PathBuf,
    native_home: PathBuf,
    python: String,
}
impl HermesScope {
    fn new() -> Self {
        let scratch = Scratch::new(false);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(scratch.state.join("setup"))
            .unwrap();
        // Resolve the interpreter before narrowing PATH to synthetic executables.
        let mut python = scrubbed_command("python3");
        python
            .args(["-c", "import sys;print(sys.executable)"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = python.spawn_owned().unwrap().wait_with_output().unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
        let interpreter = PathBuf::from(text(&output.stdout).trim())
            .canonicalize()
            .unwrap();
        let root = scratch.root.join("synthetic source");
        let package = root.join("hermes_cli");
        let dependencies = scratch.root.join("dependencies/site-packages");
        let native_home = scratch.root.join("native home with spaces");
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(&dependencies).unwrap();
        fs::create_dir_all(native_home.join("profiles/work")).unwrap();
        let py = |p: &Path| serde_json::to_string(p.to_str().unwrap()).unwrap();
        for (path, bytes) in [
            (root.join("hermes_bootstrap.py"), format!("import os,sys\nfrom pathlib import Path\n_root=Path(__file__).resolve().parent\n_pm_repair=False\n_launch_python=None\nsys.path[:]=[str(_root),{},*sys.path[1:]]\nos.environ['PYTHONPATH']=os.pathsep.join(sys.path[:2])\n", py(&dependencies))),
            (root.join("hermes_constants.py"), "import os\nfrom pathlib import Path\ndef get_hermes_home(): return Path(os.environ['HERMES_HOME'])\n".into()),
            (package.join("__init__.py"), String::new()),
            (package.join("profiles.py"), format!("from pathlib import Path\ndef normalize_profile_name(p): return p.strip().lower()\ndef validate_profile_name(p):\n if p not in ('default','work'): raise ValueError('private')\ndef resolve_profile_env(p):\n p=normalize_profile_name(p);validate_profile_name(p)\n root=Path({});home=root if p=='default' else root/'profiles'/p\n if not home.is_dir(): raise FileNotFoundError('private')\n return str(home)\n", py(&native_home))),
            (package.join("version_info.py"), "from types import SimpleNamespace\ndef get_version_info(): return SimpleNamespace(source='git',base_version='0.21.5',derived_version='0.21.5+1.g1234567',commit='1234567890abcdef1234567890abcdef12345678',dirty=False,distance=1)\n".into()),
            (package.join("config.py"), "class FailedConfigRead(dict): pass\ndef load_config_readonly(): return {'plugins':{'enabled':['herdr-threads'],'disabled':[]}}\n".into()),
        ] {
            fs::write(path, bytes).unwrap();
        }
        for home in [&native_home, native_home.join("profiles/work").as_path()] {
            fs::write(
                home.join("config.yaml"),
                b"plugins:\n  enabled: [herdr-threads]\nother: preserved\n",
            )
            .unwrap();
        }
        let bootstrap = format!(
            "import sys,runpy;sys.path.insert(0,{});import hermes_bootstrap;runpy.run_module('trace',run_name='__main__',alter_sys=True)",
            py(&root)
        );
        let launcher = scratch.root.join("selected launcher");
        let calls = scratch.root.join("selected-launcher-calls.jsonl");
        fs::write(&launcher, format!("#!{}\nimport json,sys,os\nwith open({},'a') as f: f.write(json.dumps({{'argv':sys.argv[1:],'home':os.environ['HOME'],'path':os.environ.get('PATH'),'native_home':os.environ.get('HERMES_HOME')}})+'\\n')\nprint(json.dumps([{},'-I','-c',{},'--count','--no-report',sys.argv[-3],'--profile',sys.argv[-1]]))\n", interpreter.display(), py(&calls), py(&interpreter), serde_json::to_string(&bootstrap).unwrap())).unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            python: interpreter.display().to_string(),
            scratch,
            launcher,
            native_home,
        }
    }

    fn norm(&self) -> Norm {
        self.scratch.norm().also(self.python.clone(), "<python>")
    }

    /// `verb hermes --profile work --harness-binary <launcher>`, in `form`.
    fn run(&self, verb: &str, json: bool) -> Output {
        let mut command = self.scratch.command();
        command
            .env("PATH", &self.scratch.bin)
            .env("HERMES_HOME", &self.native_home);
        let mut args = Vec::new();
        if json {
            args.push("--json");
        }
        args.extend([verb, "hermes", "--profile", "work"]);
        self.scratch.instance_args(&mut command, &args);
        command.arg("--harness-binary").arg(&self.launcher);
        command.output().unwrap()
    }
}

// ---------------------------------------------------------------------------
// setup / setup-status / unsetup
// ---------------------------------------------------------------------------

/// A scratch with claude and codex on PATH.
fn fresh() -> Scratch {
    let s = Scratch::new(false);
    s.harnesses();
    s
}

const FORMS: [(&str, &[&str]); 2] = [("text", &[]), ("json", &["--json"])];

fn with_form(form: &[&str], rest: &[&str]) -> Vec<String> {
    form.iter().chain(rest).map(|s| (*s).to_owned()).collect()
}

fn run_args(s: &Scratch, args: &[String]) -> Output {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    s.run(&refs)
}

#[test]
fn setup_reports() {
    for harness in ["claude", "codex"] {
        for (ext, form) in FORMS {
            let s = fresh();
            let out = run_args(&s, &with_form(form, &["setup", harness]));
            record(&s.norm(), &format!("setup/{harness}.{ext}"), &out);
        }
    }
    for (ext, form) in FORMS {
        let s = fresh();
        let out = run_args(&s, &with_form(form, &["setup"]));
        record(&s.norm(), &format!("setup/all.{ext}"), &out);
    }
    for (ext, json) in [("text", false), ("json", true)] {
        let h = HermesScope::new();
        let out = h.run("setup", json);
        record(&h.norm(), &format!("setup/hermes.{ext}"), &out);
    }
}

#[test]
fn setup_status_reports() {
    for harness in ["claude", "codex"] {
        for (ext, form) in FORMS {
            let s = fresh();
            let out = run_args(&s, &with_form(form, &["setup-status", harness]));
            record(
                &s.norm(),
                &format!("setup-status/{harness}.absent.{ext}"),
                &out,
            );
            s.ok(&["setup", harness]);
            let out = run_args(&s, &with_form(form, &["setup-status", harness]));
            record(&s.norm(), &format!("setup-status/{harness}.{ext}"), &out);
        }
    }
    for (ext, form) in FORMS {
        let s = fresh();
        s.ok(&["setup", "claude"]);
        s.ok(&["setup", "codex"]);
        let out = run_args(&s, &with_form(form, &["setup-status"]));
        record(&s.norm(), &format!("setup-status/all.{ext}"), &out);
    }
    for (ext, json) in [("text", false), ("json", true)] {
        let h = HermesScope::new();
        let out = h.run("setup-status", json);
        record(
            &h.norm(),
            &format!("setup-status/hermes.absent.{ext}"),
            &out,
        );
        let installed = h.run("setup", true);
        assert!(installed.status.success(), "{}", text(&installed.stderr));
        let out = h.run("setup-status", json);
        record(&h.norm(), &format!("setup-status/hermes.{ext}"), &out);
    }
}

#[test]
fn unsetup_reports() {
    for harness in ["claude", "codex"] {
        for (ext, form) in FORMS {
            let s = fresh();
            let out = run_args(&s, &with_form(form, &["unsetup", harness]));
            record(&s.norm(), &format!("unsetup/{harness}.absent.{ext}"), &out);
            s.ok(&["setup", harness]);
            let out = run_args(&s, &with_form(form, &["unsetup", harness]));
            record(&s.norm(), &format!("unsetup/{harness}.{ext}"), &out);
        }
    }
    for (ext, json) in [("text", false), ("json", true)] {
        let h = HermesScope::new();
        let installed = h.run("setup", true);
        assert!(installed.status.success(), "{}", text(&installed.stderr));
        let out = h.run("unsetup", json);
        record(&h.norm(), &format!("unsetup/hermes.{ext}"), &out);
    }
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

#[test]
fn doctor_reports() {
    // Claude installed, Codex on PATH but not installed, Hermes and the
    // synthetic fourth unresolved: the daemon is not running.
    for (name, form) in [
        ("text", &[][..]),
        ("debug.text", &["--debug"][..]),
        ("json", &["--json"][..]),
    ] {
        let s = fresh();
        s.ok(&["setup", "claude", "--keep-prompt-suggestions"]);
        let mut args = with_form(form, &["doctor"]);
        if name == "debug.text" {
            args = vec!["doctor".into(), "--debug".into()];
        }
        let out = run_args(&s, &args);
        record(&s.norm(), &format!("doctor/all.{name}"), &out);
    }
    for (ext, json) in [("text", false), ("json", true)] {
        let h = HermesScope::new();
        let installed = h.run("setup", true);
        assert!(installed.status.success(), "{}", text(&installed.stderr));
        // A link to the launcher setup already executed, not a fresh copy:
        // macOS assesses a new file's first exec (up to seconds under load),
        // which pushed the doctor's Hermes probe past its bound.
        std::os::unix::fs::symlink(&h.launcher, h.scratch.bin.join("hermes")).unwrap();
        let mut command = h.scratch.command();
        command
            .env("PATH", &h.scratch.bin)
            .env("HERMES_HOME", &h.native_home);
        let mut args = vec![];
        if json {
            args.push("--json");
        }
        args.extend(["doctor", "--harness", "hermes", "--profile", "work"]);
        h.scratch.instance_args(&mut command, &args);
        let out = command.output().unwrap();
        record(&h.norm(), &format!("doctor/hermes.{ext}"), &out);
    }
}

// ---------------------------------------------------------------------------
// help texts
// ---------------------------------------------------------------------------

#[test]
fn help_texts() {
    let s = Scratch::new(false);
    for (name, args) in [
        ("setup", &["setup", "--help"][..]),
        ("launch", &["launch", "--help"][..]),
        ("handoff", &["handoff", "--help"][..]),
        ("me-init", &["me", "init", "--help"][..]),
    ] {
        let mut command = s.command();
        let out = command.args(args).output().unwrap();
        assert!(out.status.success(), "{args:?}");
        record(&s.norm(), &format!("help/{name}.txt"), &out);
    }
}

// ---------------------------------------------------------------------------
// contract ids
// ---------------------------------------------------------------------------

#[test]
fn contract_id_output() {
    let s = Scratch::new(false);
    for (name, args) in [
        ("text.txt", &["contract-id"][..]),
        ("json.json", &["contract-id", "--json"][..]),
    ] {
        let out = s.command().args(args).output().unwrap();
        assert!(out.status.success(), "{args:?}");
        record(&s.norm(), &format!("contract-id/{name}"), &out);
    }
}

// ---------------------------------------------------------------------------
// env markers
// ---------------------------------------------------------------------------

/// `human me init` in a pane with each agent marker variable set: whether the
/// marker is refused as agent evidence, and the exact refusal. (The
/// output-form detection, `CLAUDECODE`/`CODEX_THREAD_ID` forcing the machine
/// form on a terminal, needs a PTY; a pipe is always the machine form, which
/// the other fixtures pin.)
#[test]
fn env_marker_detection() {
    let s = Scratch::new(false);
    let cases: [(&str, Vec<(&str, &str)>); 11] = [
        ("none", vec![]),
        ("CLAUDECODE", vec![("CLAUDECODE", "1")]),
        ("CLAUDECODE-empty", vec![("CLAUDECODE", "")]),
        ("CODEX_THREAD_ID", vec![("CODEX_THREAD_ID", "thread-1")]),
        ("CODEX_SANDBOX", vec![("CODEX_SANDBOX", "seatbelt")]),
        (
            "CODEX_SANDBOX_NETWORK_DISABLED",
            vec![("CODEX_SANDBOX_NETWORK_DISABLED", "1")],
        ),
        (
            "CLAUDECODE+CODEX_SANDBOX",
            vec![("CLAUDECODE", "1"), ("CODEX_SANDBOX", "seatbelt")],
        ),
        ("CODEX_HOME-only", vec![("CODEX_HOME", "/nowhere")]),
        // Agent harnesses with no adapter (Pi, OpenCode); AI_AGENT, which Claude Code also
        // sets, is no marker.
        ("PI_CODING_AGENT", vec![("PI_CODING_AGENT", "true")]),
        ("OPENCODE", vec![("OPENCODE", "1")]),
        ("AI_AGENT-only", vec![("AI_AGENT", "pi")]),
    ];
    let mut results = Vec::new();
    for (label, vars) in cases {
        let mut command = s.command();
        command.arg("human");
        s.instance_args(&mut command, &["me", "init"]);
        command.env("HERDR_PANE_ID", "w1:p1");
        for (key, value) in &vars {
            command.env(key, value);
        }
        let out = command.output().unwrap();
        results.push(json!({
            "case": label,
            "env": vars.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
            "exit": out.status.code(),
            "stdout": s.norm().apply(&text(&out.stdout)),
            "stderr": s.norm().apply(&text(&out.stderr)),
        }));
    }
    golden(
        "env-markers/detection.json",
        &pretty(&Value::Array(results)),
    );
}

// ---------------------------------------------------------------------------
// hook stdout per event, health JSON
// ---------------------------------------------------------------------------

fn hook_payload(event: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "hook_event_name": event,
        "session_id": "s1",
        "turn_id": "t1",
        "cwd": "/x",
        "tool_name": "Bash",
        "tool_input": {"command": "ls"},
        "tool_use_id": "tu1",
        "source": "startup",
        "transcript_path": "/t",
        "permission_mode": "default",
    }))
    .unwrap()
}

/// `herdr-threads hook <harness> --event <event>` as installed hooks run it:
/// native JSON on stdin, pane identity from HERDR_*, against a started
/// daemon whose host endpoint does not exist (so the check-in context is the
/// daemon-independent "unavailable" text).
#[test]
fn hook_stdout_per_event() {
    let s = Scratch::new(true);
    s.harnesses();
    let _daemon = DaemonGuard::start(&s);
    for harness in ["claude", "codex", "hermes"] {
        for event in [
            "SessionStart",
            "PreToolUse",
            "PostToolUse",
            "UserPromptSubmit",
            "Stop",
        ] {
            let input = s.root.join("hook-input.json");
            fs::write(&input, hook_payload(event)).unwrap();
            let mut command =
                s.configure_command(herdr_threads::test_support::harness_shim::under_harness(
                    harness,
                    Path::new(BIN),
                    &[],
                    &[],
                ));
            s.instance_args(&mut command, &["hook", harness, "--event", event]);
            command
                .env("HERDR_ENV", "1")
                .env("HERDR_PANE_ID", "p1")
                .env("HERDR_SOCKET_PATH", s.host())
                // An undeclared registration can exit before reading stdin. A private file
                // preserves the native payload without racing that refusal or losing its output.
                .stdin(Stdio::from(fs::File::open(&input).unwrap()))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let out = command.spawn_owned().unwrap().wait_with_output().unwrap();
            record(&s.norm(), &format!("hook/{harness}.{event}.stdout"), &out);
        }
    }
}

/// `daemon health` once the daemon's first boot observations have landed:
/// the host probed (reachability no longer `unknown`) and every harness's
/// executable observed. Before that, Health is the designed `starting` state
/// (`HEALTH_STARTING_NOTE` first in `notes`, unobserved harnesses as pending
/// limitations): answering `daemon health` (`DaemonGuard::start`) does not
/// mean those asynchronous lanes have run, and on a loaded host the first
/// sample can precede them. Startup-grace expiry cannot satisfy this wait:
/// an unprobed host stays `unknown`.
fn observed_health(s: &Scratch) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let out = s.ok(&["--json", "daemon", "health"]);
        let health: Value = serde_json::from_slice(&out.stdout).unwrap();
        let data = &health["result"]["data"];
        let lines = |key: &str| -> Vec<String> {
            data[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line.as_str().unwrap().to_owned())
                .collect()
        };
        let observed = data["host"]["reachability"] != "unknown"
            && !lines("notes")
                .iter()
                .any(|note| note == herdr_threads::protocol::results::HEALTH_STARTING_NOTE)
            && !lines("limitations").iter().any(|line| {
                herdr_threads::daemon::health::is_pending_observation_line(line)
                    || line == herdr_threads::daemon::health::FIRST_RECONCILIATION_OVERDUE_LINE
            });
        if observed {
            return health;
        }
        assert!(
            Instant::now() < deadline,
            "daemon boot observations never landed: {}",
            text(&out.stdout)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The deterministic part of `daemon health --json`: the harness map and the
/// notes (the rest carries boot ids, clocks and retry counters). Unobserved:
/// no harness executable on the daemon's PATH. Observed: fake claude, codex
/// and hermes on it.
#[test]
fn health_json_observed_unobserved() {
    for (name, observed) in [("unobserved", false), ("observed", true)] {
        let s = Scratch::new(true);
        if observed {
            s.harnesses();
            s.harness("hermes", "Hermes Agent v0.21.5");
        }
        let _daemon = DaemonGuard::start(&s);
        let health = observed_health(&s);
        let data = &health["result"]["data"];
        let projected = json!({
            "protocol_version": data["protocol_version"],
            "database": data["database"],
            "schema": data["schema"],
            "harness": data["harness"],
            "settings": data["settings"],
            "notes": data["notes"],
        });
        golden(
            &format!("health/{name}.json"),
            &s.norm().apply(&pretty(&projected)),
        );
    }
}

// ---------------------------------------------------------------------------
// launch and handoff: the topology fixture (tests/handoff_topology_cli.rs)
// ---------------------------------------------------------------------------

const BODY: &str = "golden durable work\n--agent-arg=body stays here";
const OPTIONS: &str = "--model 'env model' --config 'note=\"literal $HOME\"' --config repeat=true";
const WAIT: Duration = Duration::from_secs(60);

fn wait(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn pane(id: &str, agent: Option<&Value>) -> Value {
    let (space, tab, terminal) = match id {
        "w4:p1" => ("w4", "w4:t1", "term_1"),
        "w4:p2" => ("w4", "w4:t1", "term_2"),
        "w4:p3" => ("w4", "w4:t2", "term_3"),
        "w5:p1" => ("w5", "w5:t1", "term_focus"),
        _ => panic!("unexpected private pane {id}"),
    };
    let mut value = json!({"pane_id":id,"terminal_id":terminal,"workspace_id":space,"tab_id":tab,"focused":id=="w5:p1","revision":1,"agent_status":"idle"});
    if let Some(agent) = agent {
        value["agent"] = agent["agent"].clone();
        value["agent_status"] = json!("working");
    }
    value
}

#[derive(Default)]
struct HostState {
    requests: Vec<Value>,
    created: bool,
    agent: Option<Value>,
}

/// A model-free Herdr host on a Unix socket: the topology of
/// `handoff_topology_cli` (caller workspace w4 with the sender and recipient
/// panes, a focused workspace w5) and a native `agent.start` that records its
/// request and starts nothing.
struct Host {
    state: Arc<Mutex<HostState>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    socket: PathBuf,
}
impl Host {
    fn start(socket: &Path) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        let state = Arc::new(Mutex::new(HostState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stopping) = (state.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) => panic!("owned host accept: {e}"),
                };
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut line = String::new();
                if BufReader::new(&mut stream).read_line(&mut line).is_err() {
                    continue;
                }
                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let mut state = shared.lock().unwrap();
                state.requests.push(request.clone());
                let method = request["method"].as_str().unwrap();
                let result = match method {
                    "ping" => json!({"type":"pong","version":"0.9.1","protocol":22}),
                    "session.snapshot" => {
                        let mut tabs = vec![
                            json!({"tab_id":"w4:t1","workspace_id":"w4","label":"Original"}),
                            json!({"tab_id":"w5:t1","workspace_id":"w5","label":"Focused"}),
                        ];
                        let mut panes = vec![
                            pane("w4:p1", None),
                            pane(
                                "w4:p2",
                                state.agent.as_ref().filter(|a| a["pane_id"] == "w4:p2"),
                            ),
                            pane("w5:p1", None),
                        ];
                        if state.created {
                            tabs.push(json!({"tab_id":"w4:t2","workspace_id":"w4","label":"Peer"}));
                            panes.push(pane(
                                "w4:p3",
                                state.agent.as_ref().filter(|a| a["pane_id"] == "w4:p3"),
                            ));
                        }
                        json!({"type":"session_snapshot","snapshot":{"version":"0.9.1","protocol":22,"agents":state.agent.iter().collect::<Vec<_>>(),"layouts":[],"workspaces":[{"workspace_id":"w4","label":"Caller"},{"workspace_id":"w5","label":"Other","focused":true}],"tabs":tabs,"panes":panes}})
                    }
                    "pane.get" => {
                        let id = request["params"]["pane_id"].as_str().unwrap();
                        json!({"type":"pane_info","pane":pane(id,state.agent.as_ref().filter(|a|a["pane_id"]==id))})
                    }
                    "tab.create" => {
                        state.created = true;
                        json!({"type":"tab_created","tab":{"tab_id":"w4:t2","workspace_id":"w4"},"root_pane":pane("w4:p3",None)})
                    }
                    "agent.start" => {
                        let kind = request["params"]["kind"].as_str().unwrap();
                        let mut agent = pane(request["params"]["pane_id"].as_str().unwrap(), None);
                        agent["agent"] = json!(kind);
                        agent["agent_status"] = json!("working");
                        agent["name"] = request["params"]["name"].clone();
                        agent["interactive_ready"] = json!(true);
                        state.agent = Some(agent.clone());
                        json!({"type":"agent_started","argv":request["params"]["args"],"agent":agent})
                    }
                    "agent.get" => json!({"type":"agent_info","agent":state.agent}),
                    // The started agent: the one foreground process besides the shell (session-keyed
                    // bindings record the launched process's fingerprint); the test process stands for it.
                    "pane.process_info" => json!({"type":"pane_process_info","process_info":{
                        "pane_id":request["params"]["pane_id"],"shell_pid":1,
                        "foreground_processes":[{"pid":std::process::id(),"name":"agent","argv0":"agent"}]
                    }}),
                    other => panic!("unexpected private native request {other}: {request}"),
                };
                let _ = writeln!(stream, "{}", json!({"id":request["id"],"result":result}));
            }
        });
        Self {
            state,
            stop,
            worker: Some(worker),
            socket: socket.to_path_buf(),
        }
    }
    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = UnixStream::connect(&self.socket);
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            let _ = writeln!(std::io::stderr(), "owned host panicked during cleanup");
        }
    }
    fn requests(&self, method: &str) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r["method"] == method)
            .cloned()
            .collect()
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

struct Topology {
    daemon: OwnedChild,
    host: Host,
    iso: TestIsolation,
    context: RuntimeContext,
    paths: NodePaths,
    instance: herdr_threads::node::NodeUuid,
}
impl Topology {
    fn new() -> Self {
        let iso = TestIsolation::new("golden-surfaces");
        for dir in ["claude-config", "codex-home", "cwd"] {
            fs::create_dir(iso.path(dir)).unwrap();
        }
        // The fake host starts nothing; the binaries only have to resolve.
        for kind in ["claude", "codex"] {
            let path = iso.path(kind);
            fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let socket = iso.socket_path("host.sock");
        let host = Host::start(&socket);
        let state = iso.path("state");
        fs::create_dir_all(&state).unwrap();
        let context = RuntimeContext::explicit(state, socket, None).unwrap();
        let paths = NodePaths::resolve(&context.state_dir).unwrap();
        let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
        let instance = owner.node_uuid().unwrap();
        drop(owner);
        let seeded_epoch = Self::seed(&paths, &context, instance);
        let mut cmd = iso.command(BIN);
        cmd.args(["daemon", "run", "--state-dir"])
            .arg(&context.state_dir)
            .arg("--host-endpoint")
            .arg(context.host_endpoint.clone().unwrap())
            .env("CLAUDE_CONFIG_DIR", iso.path("claude-config"))
            .env("CODEX_HOME", iso.path("codex-home"))
            .stdout(fs::File::create(iso.path("daemon.stdout")).unwrap())
            .stderr(fs::File::create(iso.path("daemon.stderr")).unwrap());
        let mut daemon = cmd.spawn_owned().unwrap();
        // The seed publishes before any seat exists, so its empty recovery
        // completes at publication and the reconciled marker already matches
        // before the daemon has observed anything. Wait for the daemon's own,
        // newer epoch instead: its startup observation and its reconciliation
        // of the seeded seats finish before a test reads or edits them.
        let deadline = Instant::now() + WAIT;
        let mut last = String::from("no descriptor");
        loop {
            assert!(daemon.try_wait().unwrap().is_none(), "owned daemon exited");
            if paths.descriptor_path.exists() {
                let row = Connection::open(&paths.database_path).and_then(|db| {
                    db.query_row(
                        "SELECT recovery_epoch,reconciled_epoch,reconciled_boot IS NOT NULL AND reconciled_boot=recovery_boot FROM herdr_hosts",
                        [],
                        |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<bool>>(2)?)),
                    )
                });
                if let Ok((Some(recovery), Some(reconciled), Some(true))) = row
                    && recovery > seeded_epoch as i64
                    && reconciled == recovery
                {
                    break;
                }
                last = format!("(recovery_epoch, reconciled_epoch, boot_matches)={row:?}");
            }
            assert!(
                Instant::now() < deadline,
                "timed out: owned elected daemon reconciliation of an epoch after the seed's {seeded_epoch}; last {last}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        Self {
            daemon,
            host,
            iso,
            context,
            paths,
            instance,
        }
    }

    /// Seeds the host snapshot and both seats; returns the seed's epoch.
    fn seed(
        paths: &NodePaths,
        context: &RuntimeContext,
        instance: herdr_threads::node::NodeUuid,
    ) -> u64 {
        Self::seed_with_store_clock(paths, context, instance, Arc::new(SystemClock::new())).epoch
    }

    /// Seeds the instance's store. `store_clock` is the clock the store's
    /// writer quantum and budgets read; the host call keeps the system clock.
    /// Returns the seed's published epoch and how many writer turns staged
    /// the snapshot's targets.
    fn seed_with_store_clock(
        paths: &NodePaths,
        context: &RuntimeContext,
        instance: herdr_threads::node::NodeUuid,
        store_clock: Arc<dyn Clock>,
    ) -> Seeded {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let native = NativeCli::new(context.host_endpoint.clone().unwrap(), clock.clone());
        let ctx = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(clock.monotonic_now().0 + 5000),
                cancellation: Cancellation::default(),
            },
            expected_boot: None,
            expected_epoch: None,
        };
        let observed = native
            .observe_bootstrap_target(
                &herdr_threads::protocol::ids::HostTargetId::new("w4:p1"),
                &ctx,
            )
            .unwrap();
        let observation = observed.observation();
        let store_context = StoreContext::new(paths.database_path.clone(), store_clock.clone())
            .with_node_seed(herdr_threads::test_support::node_seed_for(instance));
        let budget = CallBudget {
            deadline: MonoInstant(store_clock.monotonic_now().0 + 5000),
            cancellation: Cancellation::default(),
        };
        // The host row the elected daemon will attach (same endpoint).
        store_context
            .attach_host(&context.host_endpoint.clone().unwrap().to_string_lossy())
            .unwrap();
        let db = store_context.open_writer().unwrap();
        let store = SqliteStore::new(
            store_context,
            instance.to_string(),
            StoreSettings::default(),
        )
        .unwrap();
        let admission = store
            .begin_host_observation(
                &instance.to_string(),
                herdr_threads::test_support::test_host_id(),
                &budget,
            )
            .unwrap();
        let snapshot = herdr_threads::ports::HostPort::enumerate_targets(&native, &ctx).unwrap();
        let stage = store
            .begin_snapshot_stage(
                SnapshotHeader::from_captured(admission, &snapshot).unwrap(),
                &budget,
            )
            .unwrap();
        // A writer turn that spends its quantum (a scheduling stall on a
        // loaded machine) commits only a prefix; stage the rest in the
        // following turns, as `publish_captured` does.
        let admission = DurableWorkAdmission::new(16).unwrap();
        let mut turns = 0;
        let mut offset = 0usize;
        while offset < snapshot.targets.len() {
            let progress = store
                .stage_snapshot_targets(
                    &stage.id,
                    offset as u64,
                    &snapshot.targets[offset..],
                    admission,
                    &budget,
                )
                .unwrap();
            assert!(progress.visited > 0, "a staging turn must advance");
            offset += usize::from(progress.visited);
            turns += 1;
        }
        store.seal_snapshot_stage(&stage.id, &budget).unwrap();
        let epoch = store
            .publish_snapshot_stage(&stage.id, &budget)
            .unwrap()
            .epoch;
        let execution = uuid::Uuid::new_v4();
        for (seat, target, terminal) in [
            ("sender", "w4:p1", "term_1"),
            ("recipient", "w4:p2", "term_2"),
        ] {
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_host_boot,structural_host_epoch,structural_incarnation_kind,structural_connection_epoch,structural_observation_sequence,created_at,home_node,is_local,host_id) VALUES(?1,?2,'resolved','native',?3,1,1,?4,?5,?5,?6,'native_current_target',?6,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",params![seat,instance.to_string(),target,terminal,observation.host_boot.as_str(),epoch as i64]).unwrap();
        }
        // The codex occupant's process is this test process, so the daemon's liveness worker
        // keeps the binding connected (deterministic availability across the epoch carry).
        let process = herdr_threads::protocol::authority::LivenessFingerprint::current_process();
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,liveness_pid,liveness_start) VALUES('sender',1,'w4:p1',?1,?5,1,'codex','session',?2,'cooperative_top_level',0,0,'term_1',?1,?3,?4)",params![observation.host_boot.as_str(),execution.to_string(),i64::from(process.pid),process.start as i64,epoch as i64]).unwrap();
        let contexts = paths
            .node_dir
            .join("contexts")
            .join(format!("{:x}", Sha256::digest(b"sender")));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&contexts)
            .unwrap();
        fs::set_permissions(&contexts, fs::Permissions::from_mode(0o700)).unwrap();
        ContextJournal::open(&contexts, instance, "sender", Duration::from_secs(1))
            .map(|journal| journal.with_host(herdr_threads::test_support::test_host_locator()))
            .unwrap()
            .install_reattached(OccupantContext {
                format_version: 1,
                instance,
                seat: "sender".into(),
                target: "w4:p1".into(),
                harness: Harness::Codex,
                binding_generation: 1,
                execution,
                session: SessionReference::Native("session".into()),
                role: Role::TopLevel,
            })
            .unwrap();
        Seeded { epoch, turns }
    }

    /// The roots, the instance, and the IDs the daemon generated for this
    /// run (threads, invitations, messages and the seats a handoff created): random per run, so
    /// rewritten to `<thread>`, `<invitation>`, `<message>` and `<seat>` (the fixture's own seats, `sender`
    /// and `recipient`, stay).
    fn norm(&self) -> Norm {
        let sockets = self
            .context
            .host_endpoint
            .clone()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let mut norm = Norm::new(&[self.iso.state_root(), &sockets])
            .also(self.instance.to_string(), "<instance>");
        if let Ok(db) = Connection::open_with_flags(
            &self.paths.database_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            let ids = |sql: &str| -> Vec<String> {
                db.prepare(sql)
                    .and_then(|mut q| q.query_map([], |r| r.get(0))?.collect())
                    .unwrap_or_default()
            };
            for id in ids("SELECT id FROM threads") {
                norm = norm.also(id, "<thread>");
            }
            for id in ids("SELECT id FROM invitations") {
                norm = norm.also(id, "<invitation>");
            }
            for id in ids("SELECT id FROM messages") {
                norm = norm.also(id, "<message>");
            }
            for id in ids("SELECT id FROM seats WHERE id NOT IN ('sender','recipient')") {
                norm = norm.also(id, "<seat>");
            }
        }
        norm
    }

    fn command(&self, json: bool) -> Command {
        let mut cmd = self.iso.command(BIN);
        if json {
            cmd.arg("--json");
        }
        let mut path = vec![self.iso.path("")];
        path.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        cmd.arg("--state-dir")
            .arg(&self.context.state_dir)
            .arg("--host-endpoint")
            .arg(self.context.host_endpoint.clone().unwrap())
            .env("CLAUDE_CONFIG_DIR", self.iso.path("claude-config"))
            .env("CODEX_HOME", self.iso.path("codex-home"))
            .env("PATH", std::env::join_paths(path).unwrap())
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .current_dir(self.iso.path("cwd"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    fn finish(&self, mut cmd: Command) -> Output {
        let mut child = cmd.spawn_owned().unwrap();
        wait("owned CLI exit", || child.try_wait().unwrap().is_some());
        child.wait_with_output().unwrap()
    }

    /// `args` as the cooperative top-level `sender` seat.
    fn run(&self, args: &[&str], json: bool) -> Output {
        let mut cmd = self.command(json);
        cmd.args([
            "--cooperative-seat",
            "sender",
            "--cooperative-target",
            "w4:p1",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
        ])
        .args(args);
        self.finish(cmd)
    }

    /// `args` as the local operator (no caller flags).
    fn run_operator(&self, args: &[&str], json: bool) -> Output {
        let mut cmd = self.command(json);
        cmd.args(args);
        self.finish(cmd)
    }

    /// `setup <kind>` against this instance (launch and handoff refuse
    /// without the owned hooks).
    fn setup(&self, kind: &str) {
        let mut cmd = self.iso.command(BIN);
        cmd.args(["human", "--json", "--state-dir"])
            .arg(&self.context.state_dir)
            .arg("--host-endpoint")
            .arg(self.context.host_endpoint.clone().unwrap())
            .args(["setup", kind]);
        if kind == "claude" {
            cmd.arg("--keep-prompt-suggestions");
        }
        cmd.arg("--harness-binary")
            .arg(self.iso.path(kind))
            .env("CLAUDE_CONFIG_DIR", self.iso.path("claude-config"))
            .env("CODEX_HOME", self.iso.path("codex-home"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let out = self.finish(cmd);
        assert!(
            out.status.success(),
            "{}{}",
            text(&out.stdout),
            text(&out.stderr)
        );
    }

    fn binary(&self, kind: &str) -> String {
        self.iso.path(kind).display().to_string()
    }

    /// The `agent.start` request the host received, without its wall-clock
    /// `timeout_ms` (a remaining budget that differs on every run).
    fn start_params(&self) -> Value {
        let starts = self.host.requests("agent.start");
        assert_eq!(
            starts.len(),
            1,
            "{:?}",
            self.host.state.lock().unwrap().requests
        );
        let mut params = starts[0]["params"].clone();
        params.as_object_mut().unwrap().remove("timeout_ms");
        params
    }
}

/// What `Topology::seed_with_store_clock` published.
struct Seeded {
    /// The epoch of the seed's published snapshot.
    epoch: u64,
    /// How many writer turns staged the snapshot's targets.
    turns: usize,
}

impl Drop for Topology {
    fn drop(&mut self) {
        self.daemon.stop();
        self.host.stop_and_join();
    }
}

/// Advances 10 ms on every read, so the store's 5 ms writer quantum is spent
/// after each staged target: the stall a loaded machine causes (ht-01s).
struct SteppingClock(std::sync::atomic::AtomicU64);
impl Clock for SteppingClock {
    fn utc_now(&self) -> herdr_threads::protocol::time::UtcMillis {
        herdr_threads::protocol::time::UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.fetch_add(10, Ordering::AcqRel))
    }
}

/// The store ends a staging turn early when its writer quantum is spent (a
/// scheduling stall), committing only a prefix of the targets. The fixture
/// must stage the remainder in following turns, as `publish_captured` does,
/// or sealing fails with "snapshot target set incomplete" on a loaded machine.
#[test]
fn seeding_stages_the_rest_after_a_spent_writer_quantum() {
    let iso = TestIsolation::new("golden-seed");
    let socket = iso.socket_path("host.sock");
    let _host = Host::start(&socket);
    let state = iso.path("state");
    fs::create_dir_all(&state).unwrap();
    let context = RuntimeContext::explicit(state, socket, None).unwrap();
    let paths = NodePaths::resolve(&context.state_dir).unwrap();
    let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
    let instance = owner.node_uuid().unwrap();
    drop(owner);
    let clock: Arc<dyn Clock> = Arc::new(SteppingClock(std::sync::atomic::AtomicU64::new(1)));
    let turns = Topology::seed_with_store_clock(&paths, &context, instance, clock).turns;
    let db = Connection::open(&paths.database_path).unwrap();
    let staged: i64 = db
        .query_row("SELECT count(*) FROM snapshot_targets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(staged, 3, "the fake host's three panes are all staged");
    assert_eq!(turns, 3, "one target fits each spent-quantum turn");
}

#[test]
fn launch_and_handoff_reports() {
    for kind in ["claude", "codex"] {
        for (ext, json) in [("text", false), ("json", true)] {
            let t = Topology::new();
            t.setup(kind);
            let binary = t.binary(kind);
            let out = t.run_operator(
                &[
                    "launch",
                    "--kind",
                    kind,
                    "--pane",
                    "w4:p2",
                    "--harness-binary",
                    &binary,
                ],
                json,
            );
            record(&t.norm(), &format!("launch/{kind}.{ext}"), &out);
            if json {
                golden(
                    &format!("launch/{kind}.start-request.json"),
                    &t.norm().apply(&pretty(&t.start_params())),
                );
            }
        }
    }
}

/// `launch --kind hermes` and `handoff --kind hermes`: Hermes is accepted by
/// the parser; the reports record where today's code refuses it.
#[test]
fn hermes_launch_and_handoff_refusals() {
    let t = Topology::new();
    for (ext, json) in [("text", false), ("json", true)] {
        let out = t.run_operator(&["launch", "--kind", "hermes", "--pane", "w4:p2"], json);
        record(&t.norm(), &format!("launch/hermes.{ext}"), &out);
        let out = t.run(
            &[
                "handoff",
                "--pane",
                "w4:p2",
                "--new-thread",
                "--topic",
                "Golden",
                "--goal",
                "Golden",
                "--kind",
                "hermes",
                "--",
                BODY,
            ],
            json,
        );
        record(&t.norm(), &format!("handoff/hermes.{ext}"), &out);
    }
    assert!(t.host.requests("agent.start").is_empty());
}

/// `handoff` to a new tab and to an explicit launch pane, both kinds: the
/// report in each form and the bootstrap `agent.start` request (kind, name,
/// pane and the native argv with its bootstrap prompt).
///
/// Every case still gets its own fresh `Topology` and `setup`: a handoff
/// leaves its thread, invitation, new seat, occupied launch pane and
/// `agent.start` request behind, so a reused topology would change the next
/// case's report. The four (kind, target) pairs run side by side instead,
/// each on its own isolated daemon and fake host, its text then JSON case in
/// turn; no golden depends on another pair.
#[test]
fn handoff_bootstrap_plans() {
    std::thread::scope(|scope| {
        for kind in ["claude", "codex"] {
            for (mode, target) in [
                ("new-tab", ["--new-tab", "Peer"]),
                ("pane", ["--pane", "w4:p2"]),
            ] {
                scope.spawn(move || {
                    for (ext, json) in [("text", false), ("json", true)] {
                        handoff_bootstrap_plan(kind, mode, target, ext, json);
                    }
                });
            }
        }
    });
}

/// One `handoff_bootstrap_plans` case on a fresh topology.
fn handoff_bootstrap_plan(kind: &str, mode: &str, target: [&str; 2], ext: &str, json: bool) {
    let t = Topology::new();
    t.setup(kind);
    let binary = t.binary(kind);
    // Claude takes `--model` options, Codex `--config` (the
    // pairing tests/handoff_topology_cli.rs uses for a pane).
    let (flag, options) = if kind == "claude" {
        (
            "--agent-arg=--model",
            "--model 'env model' --model 'note=\"literal $HOME\"' --model repeat=true",
        )
    } else {
        ("--agent-arg=--config", OPTIONS)
    };
    let mut args = vec!["handoff"];
    args.extend(target);
    args.extend([
        "--new-thread",
        "--topic",
        "Golden topic",
        "--goal",
        "Golden goal",
        "--kind",
        kind,
        "--harness-binary",
        &binary,
        flag,
        "--agent-arg=caller=\"quoted value\"",
        "--",
        BODY,
    ]);
    let mut cmd = t.command(json);
    cmd.args([
        "--cooperative-seat",
        "sender",
        "--cooperative-target",
        "w4:p1",
        "--cooperative-harness",
        "codex",
        "--cooperative-role",
        "top-level",
    ])
    .args(&args)
    .env("HERDR_THREADS_CODEX_OPTS", options)
    .env("HERDR_THREADS_CLAUDE_OPTS", options);
    let out = t.finish(cmd);
    record(&t.norm(), &format!("handoff/{kind}.{mode}.{ext}"), &out);
    if json {
        golden(
            &format!("handoff/{kind}.{mode}.bootstrap-argv.json"),
            &t.norm().apply(&pretty(&t.start_params())),
        );
    }
}

/// The healthy hook stdout: the same installed-hook invocation as
/// `hook_stdout_per_event`, but against a daemon whose host is the fake
/// Herdr of the launch topology, so SessionStart enrolls the pane and prints
/// the per-harness check-in context instead of the "unavailable" text.
#[test]
fn hook_stdout_healthy_checkin() {
    let t = Topology::new();
    // The topology's `sender` seat (w4:p1) is a codex occupant; claude
    // enrolls on the unbound `recipient` pane.
    for (harness, pane) in [("claude", "w4:p2"), ("codex", "w4:p1")] {
        {
            let event = "SessionStart";
            let mut cmd = t.iso.command(BIN);
            cmd.arg("--state-dir")
                .arg(&t.context.state_dir)
                .arg("--host-endpoint")
                .arg(t.context.host_endpoint.clone().unwrap())
                .args(["hook", harness, "--event", event])
                .env("HERDR_ENV", "1")
                .env("HERDR_PANE_ID", pane)
                .env(
                    "HERDR_SOCKET_PATH",
                    t.context.host_endpoint.clone().unwrap(),
                )
                .env("CLAUDE_CONFIG_DIR", t.iso.path("claude-config"))
                .env("CODEX_HOME", t.iso.path("codex-home"))
                // The hook takes its liveness fingerprint from its harness process: this test
                // process stands in for it (session-keyed bindings).
                .env("HT_TEST_HARNESS", harness)
                .current_dir(t.iso.path("cwd"))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = cmd.spawn_owned().unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&hook_payload(event))
                .unwrap();
            let out = child.wait_with_output().unwrap();
            record(
                &t.norm(),
                &format!("hook-healthy/{harness}.{event}.stdout"),
                &out,
            );
        }
    }
}
