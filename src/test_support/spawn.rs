//! Owned test children (ht-p03.131). Every child a test spawns carries
//! `HT_TEST_OWNER=<this pid>`; `spawn_owned` also puts it in its own process
//! group and kills that group on Drop (also while unwinding). A detached
//! reaper per test process (scripts/lib/isolated-herdr.sh `ih_reaper_loop`)
//! stops every tagged process and tears down owned isolated Herdr roots once
//! this process is gone, however it died (exit, panic, SIGTERM, SIGKILL).
use std::{
    ffi::OsStr,
    io::{self, Read},
    ops::{Deref, DerefMut},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::Once,
    time::{Duration, Instant},
};

/// The owner tag. Deliberately not `HERDR_`-prefixed: `scrub_env`,
/// `IsolatedHerdr::command` and `ih_env_args` strip that prefix.
/// The shell fixture that holds the reaper (`ih_reaper_ensure`). Not gated like
/// `isolated_herdr`, so every test root can use this module with or without the
/// `test-support` feature.
pub const LIB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/lib/isolated-herdr.sh");
pub const OWNER_ENV: &str = "HT_TEST_OWNER";
pub const STOP_GRACE: Duration = Duration::from_secs(2);
static REAPER: Once = Once::new();

/// Start this process's reaper once. Failure is reported, never fatal.
pub fn ensure_reaper() {
    REAPER.call_once(|| {
        let mut reaper = Command::new("/bin/sh");
        reaper
            .arg("-c")
            .arg(r#". "$1"; ih_reaper_ensure "$2""#)
            .arg("sh")
            .arg(LIB)
            .arg(std::process::id().to_string())
            .env_remove(OWNER_ENV)
            .env_remove(crate::daemon::lifecycle::TEST_OWNER_PID_ENV)
            // Sourcing must not fail when herdr is absent.
            .env("HT_SKIP_HERDR_TESTS", "1")
            .env("IH_LIB", LIB)
            // Own process group: the backgrounded loop inherits it, so a kill
            // of the test's group (a runner's group timeout) never reaches it.
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match reaper.status() {
            Ok(s) if s.success() => {}
            other => eprintln!("test-reaper: could not start: {other:?}"),
        }
    });
}

/// How much a test child stretches its external wait bounds
/// ([`crate::protocol::time::external_bound`]): the suite runs many tests in
/// parallel, so a fake `herdr` script or a daemon start can take seconds.
pub const TIMEOUT_SCALE: &str = "10";

/// Tag `command` with this process as owner (and the daemon owner pid),
/// scale its external wait bounds for a loaded suite, and relax its stores'
/// commit durability.
pub fn tag(command: &mut Command) -> &mut Command {
    ensure_reaper();
    command
        .env(OWNER_ENV, std::process::id().to_string())
        .envs([crate::daemon::lifecycle::test_owner_env()]);
    // An explicit choice on the command (set, or removed to run with the
    // production bounds) wins; `tag` runs again at `spawn_owned`.
    let scale = crate::protocol::time::TEST_TIMEOUT_SCALE_ENV;
    if !command.get_envs().any(|(key, _)| key == scale) {
        command.env(scale, TIMEOUT_SCALE);
    }
    // Test children's stores skip the per-commit fsync; set or removed
    // explicitly on the command, that choice wins.
    let durability = crate::store::connection::TEST_RELAXED_DURABILITY_ENV;
    if !command.get_envs().any(|(key, _)| key == durability) {
        command.env(durability, "1");
    }
    command
}

/// `Command::new(program)` with the scrubbed, tagged test environment.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    crate::test_support::isolation::scrub_env(&mut command);
    command
}

pub trait SpawnOwned {
    fn spawn_owned(&mut self) -> io::Result<OwnedChild>;
}

impl SpawnOwned for Command {
    fn spawn_owned(&mut self) -> io::Result<OwnedChild> {
        tag(self);
        self.process_group(0);
        let child = self.spawn()?;
        // The reaper kills recorded groups once this process is gone. The tag
        // alone cannot find Apple platform binaries: `ps` hides their environment.
        let record = group_record(child.id());
        let _ = std::fs::write(&record, "");
        Ok(OwnedChild {
            child,
            stopped: false,
            record,
        })
    }
}

/// A spawned child that leads its own process group; Drop stops the group.
pub struct OwnedChild {
    child: Child,
    stopped: bool,
    record: PathBuf,
}

/// `<reaper lock dir>/g.<pgid>`: present while the group may still need a reaper.
fn group_record(pgid: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/ih-reaper.{}/g.{pgid}", std::process::id()))
}

impl OwnedChild {
    /// SIGTERM the group, wait up to `STOP_GRACE` for the leader, SIGKILL the
    /// group, reap the leader. Idempotent.
    pub fn stop(&mut self) -> Option<ExitStatus> {
        if self.stopped {
            return self.child.try_wait().ok().flatten();
        }
        self.stopped = true;
        let group = self.child.id() as libc::pid_t;
        // SAFETY: signals only the process group this child leads.
        unsafe { libc::killpg(group, libc::SIGTERM) };
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // SAFETY: as above; also reaches grandchildren still in the group.
        unsafe { libc::killpg(group, libc::SIGKILL) };
        let status = self.child.wait().ok();
        let _ = std::fs::remove_file(&self.record);
        status
    }

    /// `Child::wait_with_output` for an owned child: closes stdin, reads
    /// stdout and stderr to the end, waits, then Drop stops what is left of the
    /// group (a grandchild that kept running).
    pub fn wait_with_output(mut self) -> io::Result<Output> {
        drop(self.child.stdin.take());
        let stdout = self.child.stdout.take();
        let stderr = self.child.stderr.take();
        let err_reader = stderr.map(|mut pipe| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                pipe.read_to_end(&mut bytes).map(|_| bytes)
            })
        });
        let mut out = Vec::new();
        if let Some(mut pipe) = stdout {
            pipe.read_to_end(&mut out)?;
        }
        let err = match err_reader {
            Some(reader) => reader
                .join()
                .map_err(|_| io::Error::other("stderr reader panicked"))??,
            None => Vec::new(),
        };
        let status = self.child.wait()?;
        Ok(Output {
            status,
            stdout: out,
            stderr: err,
        })
    }
}

impl Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}

impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Marks the child process [`ran_in_own_process`] started: the named test runs
/// its body there.
const OWN_PROCESS_TEST_ENV: &str = "HT_OWN_PROCESS_TEST";

/// Runs the calling test in a child of this test binary (`--exact`, one test,
/// its own stdout/stderr and panic hook) and returns true once it passed; in
/// that child it returns false and the test runs its body. A test whose
/// in-process daemon would hold a binary-wide lock for seconds (the daemon
/// redirects the process-wide descriptors) runs beside the others instead:
/// the daemon redirects only the child's descriptors. The child inherits this
/// environment (no added timeout scale) and is an owned test child (its group
/// is stopped on Drop). Under cargo-nextest (a process per test) it returns
/// false at once. `module` is the caller's `module_path!()`.
/// Use: `if ran_in_own_process(module_path!(), "name") { return; }`.
pub fn ran_in_own_process(module: &str, test: &str) -> bool {
    let module = module.split_once("::").map_or("", |(_, rest)| rest);
    let name = if module.is_empty() {
        test.to_owned()
    } else {
        format!("{module}::{test}")
    };
    if std::env::var_os(OWN_PROCESS_TEST_ENV).is_some_and(|value| value == name.as_str()) {
        return false;
    }
    // cargo-nextest already runs every test in a process of its own.
    if std::env::var_os("NEXTEST_EXECUTION_MODE").is_some_and(|mode| mode == "process-per-test") {
        return false;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture", "--test-threads=1"])
        .env(OWN_PROCESS_TEST_ENV, &name)
        .env_remove(crate::protocol::time::TEST_TIMEOUT_SCALE_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_owned()
        .unwrap()
        .wait_with_output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stdout}");
    eprint!("{stderr}");
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed"),
        "{name} in its own process: {:?}",
        output.status
    );
    true
}
