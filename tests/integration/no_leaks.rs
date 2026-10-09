//! ht-p03.131: no test-spawned process outlives its run, whether the test
//! process panics, is sent SIGTERM or is sent SIGKILL. A probe test (ignored,
//! a no-op unless `HT_LEAK_PROBE_DIR` is set) spawns one of everything a test
//! can spawn; these tests run it in a child test binary and then check that
//! every recorded process is gone.
use herdr_threads::test_support::{
    isolated_herdr::IsolatedHerdr,
    spawn::{self, OwnedChild, SpawnOwned},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const CHECKER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/scripts/check-no-leaked-processes"
);

fn wait_for(what: &str, limit: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

/// A sleeper `ps` can read the environment of (Apple platform binaries such as
/// `/bin/sleep` hide it, which the environment tag and the checker rely on).
fn sleeper() -> std::process::Command {
    let mut command = spawn::command("python3");
    command.args(["-c", "import time; time.sleep(600)"]);
    command
}

/// The probe: spawns one process of every kind, records `<label> <pid>`, then
/// panics or hangs by `HT_LEAK_PROBE_MODE`.
#[test]
#[ignore]
fn leak_probe_child() {
    let Some(dir) = std::env::var_os("HT_LEAK_PROBE_DIR").map(PathBuf::from) else {
        return;
    };
    let mut pids = String::new();
    let mut record = |label: &str, pid: u32| pids.push_str(&format!("{label} {pid}\n"));

    let owned = sleeper().spawn_owned().expect("spawn owned sleep");
    record("owned", owned.id());

    let grandchild_file = dir.join("grandchild.pid");
    let parent = spawn::command("/bin/sh")
        .args([
            "-c",
            "python3 -c 'import time; time.sleep(600)' & echo $! > \"$1\"; wait",
            "sh",
        ])
        .arg(&grandchild_file)
        .spawn_owned()
        .expect("spawn sh with a grandchild");
    record("shell", parent.id());
    wait_for("grandchild pid file", Duration::from_secs(10), || {
        fs::read_to_string(&grandchild_file).is_ok_and(|s| s.trim().parse::<u32>().is_ok())
    });
    let grandchild: u32 = fs::read_to_string(&grandchild_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    record("grandchild", grandchild);

    // Only the reaper can stop this one: its guard is forgotten.
    let forgotten = sleeper().spawn_owned().expect("spawn forgotten sleep");
    record("forgotten", forgotten.id());
    std::mem::forget(forgotten);

    let herdr = IsolatedHerdr::new("leak_probe");
    let socket = match &herdr {
        Some(herdr) => {
            herdr.start();
            record("herdr", herdr.pid().expect("herdr server pid"));
            fs::write(dir.join("ih_root"), herdr.root().display().to_string()).unwrap();
            herdr.socket_path()
        }
        None => dir.join("no-herdr.sock"),
    };

    let state = dir.join("state");
    let ensured = crate::scrubbed_command(BIN)
        .arg("--state-dir")
        .arg(&state)
        .arg("--host-endpoint")
        .arg(&socket)
        .args(["daemon", "ensure"])
        .env("HOME", dir.join("home"))
        .output()
        .expect("run daemon ensure");
    assert!(
        ensured.status.success(),
        "daemon ensure: {}",
        String::from_utf8_lossy(&ensured.stderr)
    );
    // RuntimeContext canonicalizes aliases before spawning the daemon. Search
    // its actual argv path rather than the caller's /var or /tmp alias.
    let daemon_state = state.canonicalize().expect("canonical daemon state");
    let needle = format!("daemon run --state-dir {}", daemon_state.display());
    wait_for("detached daemon", Duration::from_secs(20), || {
        Command::new("pgrep")
            .args(["-f", &needle])
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    });
    let found = Command::new("pgrep")
        .args(["-f", &needle])
        .output()
        .expect("pgrep");
    let daemon: u32 = String::from_utf8_lossy(&found.stdout)
        .lines()
        .next()
        .and_then(|l| l.trim().parse().ok())
        .expect("daemon pid");
    record("daemon", daemon);

    fs::write(dir.join("pids"), &pids).unwrap();
    fs::write(dir.join("ready"), "").unwrap();
    match std::env::var("HT_LEAK_PROBE_MODE").as_deref() {
        Ok("panic") => panic!("injected leak-probe panic"),
        _ => std::thread::sleep(Duration::from_secs(600)),
    }
    drop((owned, parent, herdr));
}

struct Probe {
    dir: PathBuf,
    run_id: String,
    child: OwnedChild,
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn run_probe(mode: &str) -> Probe {
    let dir = PathBuf::from(format!(
        "/tmp/htlk-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    ));
    fs::create_dir_all(&dir).unwrap();
    let run_id = uuid::Uuid::new_v4().to_string();
    let out = fs::File::create(dir.join("probe.out")).unwrap();
    let err = out.try_clone().unwrap();
    let mut child = crate::scrubbed_command(std::env::current_exe().unwrap())
        .args([
            "no_leaks::leak_probe_child",
            "--ignored",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("HT_LEAK_PROBE_DIR", &dir)
        .env("HT_LEAK_PROBE_MODE", mode)
        .env("HT_LEAK_RUN_ID", &run_id)
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn_owned()
        .expect("spawn the probe test binary");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !dir.join("ready").exists() {
        if let Ok(Some(status)) = child.try_wait() {
            panic!(
                "probe exited ({status}) before ready: {}",
                fs::read_to_string(dir.join("probe.out")).unwrap_or_default()
            );
        }
        assert!(
            Instant::now() < deadline,
            "probe never became ready: {}",
            fs::read_to_string(dir.join("probe.out")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    Probe { dir, run_id, child }
}

fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes for existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Every recorded pid is gone within 15 s; the survivors are named, killed
/// (cleanup) and then fail the test. The isolated root is removed too.
fn assert_all_gone(dir: &Path) {
    let recorded = fs::read_to_string(dir.join("pids")).expect("probe wrote its pids");
    let entries: Vec<(String, i32)> = recorded
        .lines()
        .filter_map(|l| {
            let (label, pid) = l.split_once(' ')?;
            Some((label.to_owned(), pid.parse().ok()?))
        })
        .collect();
    assert!(entries.len() >= 5, "probe recorded too little: {recorded}");
    let isolated_root = fs::read_to_string(dir.join("ih_root"))
        .ok()
        .map(PathBuf::from);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut survivors;
    let mut root_left;
    loop {
        survivors = entries
            .iter()
            .filter(|(_, pid)| pid_alive(*pid))
            .cloned()
            .collect::<Vec<_>>();
        root_left = isolated_root.clone().filter(|root| root.exists());
        if (survivors.is_empty() && root_left.is_none()) || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut report = String::new();
    for (label, pid) in &survivors {
        let cmd = Command::new("ps")
            .args(["-o", "command=", "-p", &pid.to_string()])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        report.push_str(&format!("  survivor {label} pid {pid}: {cmd}\n"));
        // SAFETY: cleanup of a pid this probe recorded.
        unsafe { libc::kill(*pid, libc::SIGKILL) };
    }
    if let Some(root) = &root_left {
        report.push_str(&format!(
            "  isolated root still exists: {}\n",
            root.display()
        ));
        let _ = fs::remove_dir_all(root);
    }
    assert!(report.is_empty(), "leaked processes:\n{report}");
}

#[test]
fn injected_panic_leaves_no_processes() {
    let mut probe = run_probe("panic");
    wait_for("the probe to exit", Duration::from_secs(60), || {
        probe.child.try_wait().ok().flatten().is_some()
    });
    let status = probe.child.try_wait().unwrap().unwrap();
    assert!(!status.success(), "the panic probe must fail, got {status}");
    assert_all_gone(&probe.dir);
}

fn killed_run_leaves_no_processes(signal: i32) {
    let mut probe = run_probe("hang");
    // Only the probe pid, not its group: what `timeout` does.
    // SAFETY: signals the probe child this test spawned.
    unsafe { libc::kill(probe.child.id() as i32, signal) };
    probe.child.wait().expect("reap the probe");
    assert_all_gone(&probe.dir);
}

#[test]
fn sigterm_killed_run_leaves_no_processes() {
    killed_run_leaves_no_processes(libc::SIGTERM);
}

#[test]
fn sigkill_killed_run_leaves_no_processes() {
    killed_run_leaves_no_processes(libc::SIGKILL);
}

#[test]
fn checker_flags_live_tagged_processes_then_passes_after_reap() {
    let mut probe = run_probe("hang");
    let recorded = fs::read_to_string(probe.dir.join("pids")).unwrap();
    let forgotten = recorded
        .lines()
        .find_map(|l| l.strip_prefix("forgotten "))
        .expect("forgotten pid recorded")
        .to_owned();
    let check = |grace: &str| {
        Command::new("python3")
            .arg(CHECKER)
            .args(["--run-id", &probe.run_id, "--grace", grace])
            .args(["--exclude", &std::process::id().to_string()])
            .output()
            .expect("run the checker")
    };
    let flagged = check("0");
    let text = String::from_utf8_lossy(&flagged.stdout).into_owned();
    assert_eq!(flagged.status.code(), Some(1), "checker output: {text}");
    assert!(
        text.lines()
            .any(|l| l.starts_with("LEAK rule=run-id") && l.contains(&format!("pid={forgotten} "))),
        "the forgotten child must be flagged: {text}"
    );
    // SAFETY: signals the probe child this test spawned.
    unsafe { libc::kill(probe.child.id() as i32, libc::SIGKILL) };
    probe.child.wait().expect("reap the probe");
    assert_all_gone(&probe.dir);
    // Not "exit 0": the checker scans the whole machine, so another run's
    // orphan (a live suite in a sibling worktree) may legitimately be listed,
    // and its own grace would then spin on that orphan. Nothing of this probe
    // may be listed: single scans (no grace) are repeated for up to the
    // checker's default 10 s grace until none lists this probe.
    let ours: Vec<&str> = recorded
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(_, pid)| pid))
        .collect();
    let lists_ours = |text: &str| {
        text.lines().any(|l| {
            l.contains("rule=run-id") || ours.iter().any(|pid| l.contains(&format!("pid={pid} ")))
        })
    };
    let until = Instant::now() + Duration::from_secs(10);
    let text = loop {
        let clean = check("0");
        let text = String::from_utf8_lossy(&clean.stdout).into_owned();
        if !lists_ours(&text) || Instant::now() >= until {
            break text;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    assert!(
        !lists_ours(&text),
        "checker after reap still lists this probe: {text}"
    );
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Static guard: a test never spawns a bare `Command`. Each spawn goes through
/// `spawn_owned`, or the line (or the one above) says why it is exempt.
#[test]
fn every_test_spawn_site_is_owned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("tests"), &mut files);
    rust_files(&root.join("src/test_support"), &mut files);
    let needles = [
        ".spawn()",                                            // leak-guard: needle literal
        "Command::new(BIN)",                                   // leak-guard: needle literal
        "Command::new(env!(\"CARGO_BIN_EXE_herdr-threads\"))", // leak-guard: needle literal
    ];
    let mut offenders = Vec::new();
    for file in files {
        if file.ends_with("src/test_support/spawn.rs") {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !needles.iter().any(|n| line.contains(n)) {
                continue;
            }
            let marked = line.contains("// leak-guard:")
                || (i > 0 && lines[i - 1].contains("// leak-guard:"));
            if !marked {
                offenders.push(format!(
                    "{}:{}: {}",
                    file.strip_prefix(root).unwrap().display(),
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "unowned spawn sites (use spawn::command / spawn_owned, or add `// leak-guard: <reason>`):\n{}",
        offenders.join("\n")
    );
}
