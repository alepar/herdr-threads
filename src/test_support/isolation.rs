//! Per-test isolation (ht-p03.6, root spec §B9 Decision 2): a private temp
//! state root, a scrubbed environment for spawned binaries, a short socket
//! base (P37) and a per-test SQLite cost counter.
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

/// Inherited variables removed from every spawned binary's environment, so
/// the suite behaves the same inside a Herdr pane or a harness session.
pub const SCRUBBED_PREFIXES: &[&str] = &["HERDR_", "CLAUDE", "CODEX"];
/// `sockaddr_un.sun_path` capacity (macOS 104, Linux 108; the smaller wins).
pub const SUN_PATH_MAX: usize = 104;

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
/// `HT_TEST_OWNER` tag goes on too (`spawn::tag`, ht-p03.131).
pub fn scrub_env(command: &mut Command) -> &mut Command {
    scrub_inherited(command, std::env::vars_os().map(|(key, _)| key));
    crate::test_support::spawn::tag(command);
    command
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
}
