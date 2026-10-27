//! Per-test isolation (ht-p03.6, root spec §B9 Decision 2): a private temp
//! state root, a scrubbed environment for spawned binaries, a short socket
//! base (P37) and a per-test SQLite cost counter.
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

/// Inherited variables removed from every spawned binary's environment, so
/// the suite behaves the same inside a Herdr pane or a harness session.
/// `ZDOTDIR` too: shell startup files then resolve under the test's HOME, never
/// the developer's own zsh directory.
pub const SCRUBBED_PREFIXES: &[&str] = &["HERDR_", "CLAUDE", "CODEX", "ZDOTDIR"];
/// `sockaddr_un.sun_path` capacity (macOS 104, Linux 108; the smaller wins).
pub const SUN_PATH_MAX: usize = 104;

/// A missing fixture program disables ambient discovery without consulting the
/// real Herdr configuration. Shared by spawned commands and in-process daemons.
pub const NO_MACHINE_LIST_PROGRAM: &str = "/nonexistent/herdr-no-machine-list";
pub const MACHINE_LIST_PROGRAM_ENV: &str = "HT_TEST_MACHINE_LIST_PROGRAM";

pub fn discovery_program() -> OsString {
    std::env::var_os(MACHINE_LIST_PROGRAM_ENV).unwrap_or_else(|| NO_MACHINE_LIST_PROGRAM.into())
}

/// A private state root and short socket base for one test; both are removed
/// on drop.
pub struct TestIsolation {
    root: PathBuf,
    sockets: PathBuf,
}

fn create_private_dir(path: &Path) {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
}

impl TestIsolation {
    /// `<temp_dir>/ht-<label sanitized to [a-z0-9-], max 40 chars>-<uuid>`
    /// (created, 0700) and a short socket base `/tmp/hts-<12 hex>` (created,
    /// 0700) whose length never depends on the label.
    pub fn new(label: &str) -> Self {
        let clean: String = label
            .chars()
            .map(|c| c.to_ascii_lowercase())
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .take(40)
            .collect();
        let root = std::env::temp_dir().join(format!("ht-{clean}-{}", uuid::Uuid::new_v4()));
        let short = uuid::Uuid::new_v4().simple().to_string();
        let sockets = PathBuf::from("/tmp").join(format!("hts-{}", &short[..12]));
        create_private_dir(&root);
        create_private_dir(&sockets);
        Self { root, sockets }
    }

    pub fn state_root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.join(relative)
    }

    pub fn home(&self) -> PathBuf {
        let home = self.path("home");
        create_private_dir(&home);
        home
    }

    /// `<socket base>/<name>`; panics if the result would not fit `SUN_PATH_MAX`.
    pub fn socket_path(&self, name: &str) -> PathBuf {
        let socket = self.sockets.join(name);
        assert!(
            socket.as_os_str().len() < SUN_PATH_MAX,
            "socket path too long ({} bytes): {}",
            socket.as_os_str().len(),
            socket.display()
        );
        socket
    }

    /// `program` with every inherited `SCRUBBED_PREFIXES` variable removed and
    /// HOME / XDG_CONFIG_HOME / XDG_STATE_HOME / XDG_RUNTIME_DIR pointing under
    /// the root.
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        scrub_env(&mut command);
        let home = self.home();
        let xdg = |name: &str| {
            let dir = self.path(format!("xdg/{name}"));
            create_private_dir(&dir);
            dir
        };
        command
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", xdg("config"))
            .env("XDG_STATE_HOME", xdg("state"))
            .env("XDG_RUNTIME_DIR", xdg("runtime"));
        command
    }
}

/// Remove every inherited `SCRUBBED_PREFIXES` variable from `command`, then
/// set the one sanctioned `HERDR_` variable, the test-owner pid
/// (`daemon::lifecycle::test_owner_env`, ht-6y1), so any daemon the child
/// starts exits once this test process is gone. The
/// `HT_TEST_OWNER` tag goes on too (`spawn::tag`, ht-p03.131). Peer discovery
/// is pointed at no program (a test sets its own to override): the real
/// `herdr` on PATH would name the developer's saved machines as dial targets.
pub fn scrub_env(command: &mut Command) -> &mut Command {
    scrub_inherited(command, std::env::vars_os().map(|(key, _)| key));
    crate::test_support::spawn::tag(command);
    command.env(MACHINE_LIST_PROGRAM_ENV, NO_MACHINE_LIST_PROGRAM);
    command
}

/// Make `command` an agent of `harness` ("claude" | "codex" | "hermes") running session
/// `session`, for tests that drive the binary with `--cooperative-*` flags: the spawning test
/// process stands in for the harness process (fingerprint included) and the harness's own session
/// variable carries `session`. Real process ancestry is `harness_shim`'s job.
pub fn as_agent<'a>(command: &'a mut Command, harness: &str, session: &str) -> &'a mut Command {
    as_agent_in(command, harness, session, session)
}

/// [`as_agent`] whose harness process is the stand-in named `process`, so several sessions can
/// share one process (a server hosting the sessions of several panes, such as a Hermes gateway).
pub fn as_agent_in<'a>(
    command: &'a mut Command,
    harness: &str,
    session: &str,
    process: &str,
) -> &'a mut Command {
    let variable = match harness {
        "claude" => "CLAUDE_CODE_SESSION_ID",
        "codex" => "CODEX_SESSION_ID",
        "hermes" => "HERMES_SESSION_ID",
        // A registered adapter's declared caller shape names its session variable.
        other => crate::harness::registry::builtins()
            .registrations()
            .iter()
            .find(|registration| registration.metadata().id == other)
            .and_then(|registration| registration.caller_env())
            .map_or("HERMES_SESSION_ID", |shape| shape.session_var),
    };
    let stand_in = stand_in_process(harness, process).to_string();
    // Claude exports its own pid, and a claim prefers it: it names the stand-in too, never the
    // developer's real Claude session the test runner may sit under.
    command
        .env("CLAUDE_PID", &stand_in)
        .env("HT_TEST_HARNESS", harness)
        .env("HT_TEST_HARNESS_PID", &stand_in)
        .env(variable, session)
}

/// The liveness fingerprint of the stand-in process of `session` (see [`as_agent`]): a live
/// process of its own, so the liveness worker keeps its binding connected.
pub fn stand_in_fingerprint(
    harness: &str,
    session: &str,
) -> crate::protocol::authority::LivenessFingerprint {
    use crate::ports::ProcessInfo;
    let record = crate::host::process_table::HostProcessTable
        .process(stand_in_process(harness, session))
        .expect("the stand-in process is in the process table");
    crate::protocol::authority::LivenessFingerprint::of(&record)
}

/// The pid of a long-lived process standing in for the harness process of `session`, started on
/// first use and shared by every command of that session in this test process. Two sessions are
/// two processes, as in a real run (a shared fingerprint would make one supersede the other).
/// It is owned (`spawn_owned`): the reaper stops it once the test process is gone.
fn stand_in_process(harness: &str, session: &str) -> u32 {
    use crate::test_support::spawn::{OwnedChild, SpawnOwned};
    use std::{collections::HashMap, process::Stdio, sync::Mutex};
    static STAND_INS: Mutex<Option<HashMap<String, OwnedChild>>> = Mutex::new(None);
    let mut stand_ins = STAND_INS.lock().unwrap_or_else(|e| e.into_inner());
    stand_ins
        .get_or_insert_with(HashMap::new)
        .entry(format!("{harness}/{session}"))
        .or_insert_with(|| {
            Command::new("/bin/sleep")
                .arg("86400")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn_owned()
                .expect("start a stand-in harness process")
        })
        .id()
}

/// The ambient agent-harness environment a test child must not inherit (ht-54r.95), derived
/// from the registry: the exact names (every adapter's `agent_env_markers`, plus each caller
/// shape's `session_var` and `pid_var`) and the prefixes (each `session_var` up to and including
/// its first `_`), which cover role-witness inputs no declaration lists.
pub fn ambient_harness_vars() -> (Vec<&'static str>, Vec<String>) {
    let registry = crate::harness::registry::builtins();
    let mut names: Vec<&'static str> = registry.env_markers().map(|marker| marker.name).collect();
    let mut prefixes: Vec<String> = Vec::new();
    for (_, shape) in registry.caller_shapes() {
        names.push(shape.session_var);
        names.extend(shape.pid_var);
        if let Some(end) = shape.session_var.find('_') {
            prefixes.push(shape.session_var[..=end].to_owned());
        }
    }
    names.sort_unstable();
    names.dedup();
    prefixes.sort();
    prefixes.dedup();
    (names, prefixes)
}

/// Whether `name` is a harness config-dir variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`,
/// `HERMES_HOME`, ...): isolation, not attribution, so the prefix scrub keeps it. Dropping an
/// inherited isolated config dir would let the child fall back to the real user config.
fn is_config_dir_var(name: &str) -> bool {
    name.ends_with("_HOME") || name.ends_with("_CONFIG_DIR")
}

/// `env_remove` every ambient harness variable of `inherited` that `command` does not already
/// set or remove explicitly (an explicit choice wins). Config-dir variables are never removed by
/// prefix ([`is_config_dir_var`]).
pub(super) fn remove_ambient_harness_env(
    command: &mut Command,
    inherited: impl IntoIterator<Item = OsString>,
) {
    let (names, prefixes) = ambient_harness_vars();
    let explicit: std::collections::HashSet<OsString> =
        command.get_envs().map(|(key, _)| key.to_owned()).collect();
    for name in names {
        if !explicit.contains(OsStr::new(name)) {
            command.env_remove(name);
        }
    }
    for key in inherited {
        let lossy = key.to_string_lossy();
        if !explicit.contains(&key)
            && !is_config_dir_var(&lossy)
            && prefixes.iter().any(|prefix| lossy.starts_with(prefix))
        {
            command.env_remove(&key);
        }
    }
}

/// The removal half of [`scrub_env`] over an explicit list of inherited
/// keys, so it is testable without mutating this process's environment.
fn scrub_inherited(command: &mut Command, inherited: impl IntoIterator<Item = std::ffi::OsString>) {
    for key in inherited {
        if let Some(name) = key.to_str() {
            if SCRUBBED_PREFIXES.iter().any(|p| name.starts_with(p)) {
                command.env_remove(&key);
            }
        } else {
            // A non-UTF-8 key cannot start with an ASCII prefix we match on a
            // lossy view; check the lossy form to stay conservative.
            let lossy = key.to_string_lossy();
            if SCRUBBED_PREFIXES.iter().any(|p| lossy.starts_with(p)) {
                command.env_remove(&key);
            }
        }
    }
}

impl Drop for TestIsolation {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::remove_dir_all(&self.sockets);
    }
}

/// Per-test SQLite VM-instruction counter (units of 10 instructions).
#[derive(Debug, Default)]
pub struct CostCounter(AtomicU64);

impl CostCounter {
    pub fn units(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Unregisters the progress handler on drop, so a panicking workload never
/// leaves a dangling counter pointer registered.
struct HandlerGuard(*mut rusqlite::ffi::sqlite3);

impl Drop for HandlerGuard {
    fn drop(&mut self) {
        // SAFETY: the handle belongs to the `Connection` borrowed by
        // `count_vm_units`, which outlives this guard.
        unsafe { rusqlite::ffi::sqlite3_progress_handler(self.0, 0, None, std::ptr::null_mut()) };
    }
}

/// Run `f` with a progress handler that bumps `counter` every 10 VM
/// instructions on `db` only. The handler's user-data pointer is `counter`,
/// so concurrent calls on other connections never share a count.
pub fn count_vm_units<T>(
    db: &rusqlite::Connection,
    counter: &CostCounter,
    f: impl FnOnce() -> T,
) -> T {
    use rusqlite::ffi;
    extern "C" fn bump(data: *mut std::ffi::c_void) -> std::ffi::c_int {
        // SAFETY: `data` is the `&CostCounter` registered below, alive for the call.
        unsafe { &*(data as *const CostCounter) }
            .0
            .fetch_add(1, Ordering::SeqCst);
        0
    }
    // SAFETY: the handle belongs to `db`; the guard removes the handler before
    // returning or unwinding, and `counter` outlives the registration.
    let _guard = unsafe {
        let handle = db.handle();
        ffi::sqlite3_progress_handler(
            handle,
            10,
            Some(bump),
            counter as *const CostCounter as *mut _,
        );
        HandlerGuard(handle)
    };
    f()
}

/// A rig's argv with its pinned routing (`--state-dir S --host-endpoint H`, …). Routing leads,
/// except before an escalating command (setup, unsetup, doctor fix, internal
/// installer-integrations): an agent must write those words first, so its leading output
/// flags and the routing follow them.
pub fn routed_argv<S: AsRef<OsStr>>(routing: &[S], args: &[&str]) -> Vec<std::ffi::OsString> {
    let flags = args
        .iter()
        .take_while(|arg| matches!(**arg, "--json" | "--human" | "--machine"))
        .count();
    let rest = &args[flags..];
    let routing = routing.iter().map(|r| r.as_ref().to_os_string());
    if crate::cli::commands::ESCALATING_COMMANDS
        .iter()
        .any(|words| rest.starts_with(words))
    {
        rest.iter()
            .chain(&args[..flags])
            .map(std::ffi::OsString::from)
            .chain(routing)
            .collect()
    } else {
        routing
            .chain(args.iter().map(std::ffi::OsString::from))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_helpers_never_share_a_path() {
        let handles: Vec<_> = (0..32)
            .map(|i| {
                std::thread::spawn(move || {
                    let iso = TestIsolation::new(&format!("share-{i}"));
                    (iso.state_root().to_path_buf(), iso.socket_path("d"), iso)
                })
            })
            .collect();
        let all: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let mut roots: Vec<_> = all.iter().map(|(r, _, _)| r.clone()).collect();
        let mut sockets: Vec<_> = all.iter().map(|(_, s, _)| s.clone()).collect();
        roots.sort();
        roots.dedup();
        sockets.sort();
        sockets.dedup();
        assert_eq!(roots.len(), 32);
        assert_eq!(sockets.len(), 32);
        for (root, _, _) in &all {
            assert!(root.is_dir());
        }
        let kept: Vec<_> = all.iter().map(|(r, _, _)| r.clone()).collect();
        drop(all);
        for root in kept {
            assert!(!root.exists(), "dropped helper removes its root");
        }
    }

    #[test]
    fn spawned_env_has_no_herdr_claude_or_codex_vars() {
        let iso = TestIsolation::new("env");
        let mut cmd = iso.command("/usr/bin/env");
        cmd.env("HERDR_ADDED_AFTER", "kept-because-explicit");
        let out = cmd.output().unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        for line in text.lines() {
            let key = line.split('=').next().unwrap();
            if key == "HERDR_ADDED_AFTER" || key == crate::daemon::lifecycle::TEST_OWNER_PID_ENV {
                continue;
            }
            assert!(
                !SCRUBBED_PREFIXES.iter().any(|p| key.starts_with(p)),
                "{key} leaked"
            );
        }
        assert!(text.contains(&format!("HOME={}", iso.home().display())));
    }

    #[test]
    fn scrub_env_removes_inherited_prefixed_vars() {
        // A dirty inherited environment, given explicitly: mutating this
        // process's environment would race every test thread that spawns a
        // child or reads a variable (and is unsound; clippy.toml bans it).
        let inherited = [
            "HERDR_SCRUB_PROBE",
            "CLAUDE_PROBE",
            "CODEX_PROBE",
            "PATH",
            "HOME",
        ];
        let mut cmd = Command::new("/usr/bin/env");
        scrub_inherited(&mut cmd, inherited.map(std::ffi::OsString::from));
        let removed: Vec<_> = cmd
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        for key in ["HERDR_SCRUB_PROBE", "CLAUDE_PROBE", "CODEX_PROBE"] {
            assert!(
                removed.iter().any(|r| r == key),
                "{key} not removed: {removed:?}"
            );
        }
        assert!(
            !removed.iter().any(|r| r == "PATH" || r == "HOME"),
            "{removed:?}"
        );
    }

    #[test]
    fn socket_path_stays_under_the_unix_limit_for_a_deep_fixture() {
        let iso = TestIsolation::new(&"deep".repeat(60));
        let deep = iso.path("nested/".repeat(20));
        std::fs::create_dir_all(&deep).unwrap();
        let socket = iso.socket_path("daemon.sock");
        assert!(
            socket.as_os_str().len() < SUN_PATH_MAX,
            "{}",
            socket.display()
        );
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        drop(listener);
    }

    #[test]
    fn concurrent_cost_counters_see_only_their_own_units() {
        fn workload(rows: i64) -> rusqlite::Connection {
            let db = rusqlite::Connection::open_in_memory().unwrap();
            db.execute_batch(&format!(
                "CREATE TABLE t(x); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{rows}) INSERT INTO t SELECT x FROM n;"
            ))
            .unwrap();
            db
        }
        fn run(db: &rusqlite::Connection) -> u64 {
            let counter = CostCounter::default();
            count_vm_units(db, &counter, || {
                db.query_row("SELECT sum(x) FROM t", [], |r| r.get::<_, i64>(0))
                    .unwrap()
            });
            counter.units()
        }
        let solo_small = run(&workload(1_000));
        let solo_large = run(&workload(50_000));
        assert!(solo_large > solo_small * 10);
        std::thread::scope(|s| {
            let a = s.spawn(|| {
                let db = workload(1_000);
                (0..20).map(|_| run(&db)).collect::<Vec<_>>()
            });
            let b = s.spawn(|| {
                let db = workload(50_000);
                (0..20).map(|_| run(&db)).collect::<Vec<_>>()
            });
            assert!(a.join().unwrap().iter().all(|u| *u == solo_small));
            assert!(b.join().unwrap().iter().all(|u| *u == solo_large));
        });
    }

    #[test]
    fn ambient_harness_vars_cover_every_declared_marker_session_and_pid_var() {
        let (names, prefixes) = ambient_harness_vars();
        let registry = crate::harness::registry::builtins();
        assert!(registry.env_markers().count() > 0);
        for marker in registry.env_markers() {
            assert!(names.contains(&marker.name), "marker {}", marker.name);
        }
        let mut shapes = 0;
        for (id, shape) in registry.caller_shapes() {
            shapes += 1;
            assert!(names.contains(&shape.session_var), "{id} session_var");
            if let Some(pid_var) = shape.pid_var {
                assert!(names.contains(&pid_var), "{id} pid_var");
            }
            let first = &shape.session_var[..=shape.session_var.find('_').unwrap()];
            assert!(prefixes.iter().any(|p| p == first), "{id} prefix {first}");
        }
        assert!(shapes >= 3);
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted, names, "sorted and deduplicated");
    }

    #[test]
    fn tag_removes_ambient_harness_env_unless_set_explicitly() {
        let mut cmd = Command::new("/usr/bin/env");
        cmd.env("CLAUDE_CODE_SESSION_ID", "explicit");
        remove_ambient_harness_env(
            &mut cmd,
            [
                "CODEX_THREAD_ID",
                "CODEX_HOME",
                "CLAUDE_CONFIG_DIR",
                "HERMES_HOME",
                "HERMES_X",
                "ANTIGRAVITY_AGENT",
                "CLAUDECODE",
                "PATH",
                "HOME",
            ]
            .map(OsString::from),
        );
        let envs: std::collections::HashMap<_, _> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_str().unwrap().to_owned(), v.map(|v| v.to_owned())))
            .collect();
        for removed in [
            "CODEX_THREAD_ID",
            "HERMES_X",
            "ANTIGRAVITY_AGENT",
            "CLAUDECODE",
            "CLAUDE_PID",
            "CODEX_SESSION_ID",
            "HERMES_SESSION_ID",
            "ANTIGRAVITY_CONVERSATION_ID",
        ] {
            assert_eq!(envs.get(removed), Some(&None), "{removed} removed");
        }
        assert_eq!(
            envs.get("CLAUDE_CODE_SESSION_ID"),
            Some(&Some(OsString::from("explicit")))
        );
        assert!(!envs.contains_key("PATH") && !envs.contains_key("HOME"));
        // Config-dir variables are isolation, not attribution: an inherited isolated config dir
        // must reach the child, never be dropped so it falls back to the real user config.
        for kept in ["CODEX_HOME", "CLAUDE_CONFIG_DIR", "HERMES_HOME"] {
            assert!(!envs.contains_key(kept), "{kept} kept");
        }
    }

    #[test]
    fn tag_sets_the_ancestry_floor_unless_set_explicitly() {
        let floor = |cmd: &Command| {
            cmd.get_envs()
                .find(|(k, _)| *k == "HT_TEST_ANCESTRY_FLOOR_PID")
                .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
        };
        let mut plain = Command::new("/usr/bin/true");
        crate::test_support::spawn::tag(&mut plain);
        assert_eq!(floor(&plain), Some(Some(std::process::id().to_string())));
        let mut explicit = Command::new("/usr/bin/true");
        explicit.env("HT_TEST_ANCESTRY_FLOOR_PID", "1");
        crate::test_support::spawn::tag(&mut explicit);
        assert_eq!(floor(&explicit), Some(Some("1".to_owned())));
    }
}
