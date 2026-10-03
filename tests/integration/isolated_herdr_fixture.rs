//! ht-p03.1: self-test of the isolated named Herdr test-session fixture.
use herdr_threads::test_support::isolated_herdr::{
    Availability, HerdrState, IsolatedHerdr, SCRIPT, availability,
};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    os::unix::{fs::MetadataExt, net::UnixStream},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    process::Command,
    sync::{Barrier, Mutex},
};

fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only probes for existence.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn no_herdr_path() -> &'static str {
    let path = "/usr/bin:/bin";
    assert!(
        !path.split(':').any(|d| Path::new(d).join("herdr").exists()),
        "herdr unexpectedly present in {path}"
    );
    path
}

#[test]
fn every_helper_state_is_reachable() {
    let Some(h) = IsolatedHerdr::new("every_helper_state_is_reachable") else {
        return;
    };
    assert_eq!(h.state(), HerdrState::NeverStarted);
    assert_eq!(h.starts(), 0);
    assert_eq!(h.pid(), None);

    h.start();
    assert_eq!(h.state(), HerdrState::Up);
    assert!(h.socket_path().exists());
    let first = h.pid().expect("pid after start");
    assert!(pid_alive(first));

    h.stop();
    assert_eq!(h.state(), HerdrState::Stopped);
    assert!(!h.socket_path().exists());
    assert!(!pid_alive(first));

    h.stale_socket();
    assert_eq!(h.state(), HerdrState::StaleSocket);
    assert!(h.socket_path().exists());
    let err = UnixStream::connect(h.socket_path()).expect_err("dead server must refuse");
    assert_eq!(err.kind(), std::io::ErrorKind::ConnectionRefused);

    h.restart();
    assert_eq!(h.state(), HerdrState::Up);
    assert_eq!(h.starts(), 3);
    assert_ne!(h.pid().expect("pid after restart"), first);
    assert!(UnixStream::connect(h.socket_path()).is_ok());
    assert!(
        h.env()
            .iter()
            .any(|(k, v)| k == "HERDR_SOCKET_PATH" && Path::new(v) == h.socket_path())
    );
}

#[test]
fn teardown_runs_after_a_panicking_test() {
    let seen: Mutex<Option<(u32, PathBuf)>> = Mutex::new(None);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = IsolatedHerdr::new("teardown_runs_after_a_panicking_test") else {
            return;
        };
        h.start();
        *seen.lock().unwrap() = Some((h.pid().unwrap(), h.root().to_path_buf()));
        panic!("deliberate panic to prove teardown on unwind");
    }));
    let Some((pid, root)) = seen.into_inner().unwrap() else {
        return; // skipped: no herdr
    };
    assert!(result.is_err(), "the closure must have panicked");
    assert!(!pid_alive(pid), "server {pid} survived the panic");
    assert!(!root.exists(), "root {} survived the panic", root.display());
}

type Snapshot = (BTreeMap<u32, String>, (bool, u64));

fn shared_snapshot() -> Snapshot {
    let ps = Command::new("ps")
        .args(["-axo", "pid=,lstart=,command="])
        .output()
        .expect("ps");
    let mut servers = BTreeMap::new();
    for line in String::from_utf8_lossy(&ps.stdout).lines() {
        if !line.trim_end().ends_with("herdr server") {
            continue;
        }
        let line = line.trim_start();
        let Some((pid, rest)) = line.split_once(' ') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        let env = Command::new("ps")
            .args(["eww", "-o", "command=", "-p", &pid.to_string()])
            .output()
            .expect("ps eww");
        let env = String::from_utf8_lossy(&env.stdout);
        // Empty: the process exited between the two `ps` calls (a private
        // server being torn down by a parallel test), so it is not shared.
        if env.trim().is_empty() || env.contains("/tmp/ih.") {
            continue;
        }
        servers.insert(pid, rest.trim().to_owned());
    }
    let socket = std::env::var_os("HERDR_SOCKET_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME").map(|d| Path::new(&d).join("herdr/herdr.sock"))
        })
        .or_else(|| {
            std::env::var_os("HOME").map(|d| Path::new(&d).join(".config/herdr/herdr.sock"))
        });
    let meta = socket.and_then(|p| std::fs::symlink_metadata(p).ok());
    (servers, (meta.is_some(), meta.map_or(0, |m| m.ino())))
}

#[test]
fn shared_server_is_untouched() {
    let before = shared_snapshot();
    {
        let Some(h) = IsolatedHerdr::new("shared_server_is_untouched") else {
            return;
        };
        h.start();
        h.stale_socket();
        h.restart();
        assert_eq!(h.state(), HerdrState::Up);
    }
    assert_eq!(shared_snapshot(), before);
}

#[test]
fn four_private_servers_run_concurrently() {
    if IsolatedHerdr::new("four_private_servers_run_concurrently").is_none() {
        return;
    }
    let barrier = Barrier::new(4);
    let fixtures: Vec<IsolatedHerdr> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                s.spawn(|| {
                    let h = IsolatedHerdr::new("four_private_servers_run_concurrently").unwrap();
                    h.start();
                    barrier.wait();
                    assert_eq!(h.state(), HerdrState::Up);
                    h
                })
            })
            .collect();
        handles.into_iter().map(|j| j.join().unwrap()).collect()
    });
    let mut sockets: Vec<_> = fixtures.iter().map(|h| h.socket_path()).collect();
    let mut pids: Vec<_> = fixtures.iter().map(|h| h.pid().unwrap()).collect();
    sockets.sort();
    sockets.dedup();
    pids.sort();
    pids.dedup();
    assert_eq!((sockets.len(), pids.len()), (4, 4));
    drop(fixtures);
    for pid in pids {
        assert!(!pid_alive(pid), "server {pid} survived drop");
    }
}

#[test]
fn availability_policy() {
    let empty = tempdir("ih-avail-empty");
    let with = tempdir("ih-avail-with");
    let bin = with.join("herdr");
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let one = OsStr::new("1");
    assert_eq!(
        availability(Some(empty.as_os_str()), None),
        Availability::Missing
    );
    assert_eq!(
        availability(Some(empty.as_os_str()), Some(OsStr::new("0"))),
        Availability::Missing
    );
    assert_eq!(
        availability(Some(empty.as_os_str()), Some(one)),
        Availability::Skip
    );
    assert_eq!(
        availability(Some(with.as_os_str()), None),
        Availability::Present(bin.clone())
    );
    // A non-executable file named herdr does not count.
    std::fs::write(empty.join("herdr"), "").unwrap();
    assert_eq!(
        availability(Some(empty.as_os_str()), None),
        Availability::Missing
    );
    let _ = std::fs::remove_dir_all(empty);
    let _ = std::fs::remove_dir_all(with);
}

fn tempdir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn source(path: &str, skip: bool, script: &str) -> std::process::Output {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", script, "sh", SCRIPT])
        .env("PATH", path)
        .env("IH_CASE", "case_x")
        .env_remove("HT_SKIP_HERDR_TESTS");
    if skip {
        cmd.env("HT_SKIP_HERDR_TESTS", "1");
    }
    cmd.output().unwrap()
}

#[test]
fn missing_herdr_fails_naming_the_binary() {
    let path = no_herdr_path();
    let out = source(path, false, r#". "$1""#);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("'herdr'"));

    let out = source(path, true, r#". "$1" && ih_start /tmp/ih.unused case_x"#);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "skipped: no herdr: case_x"
    );
    assert!(!Path::new("/tmp/ih.unused").exists());
}

#[test]
fn skip_policy_reports_the_test_name() {
    let path = no_herdr_path();
    let exe = std::env::current_exe().unwrap();
    let run = |skip: bool| {
        let mut cmd = Command::new(&exe);
        cmd.args([
            "--exact",
            "isolated_herdr_fixture::every_helper_state_is_reachable",
            "--nocapture",
        ])
        .env("PATH", path)
        .env_remove("HT_SKIP_HERDR_TESTS");
        if skip {
            cmd.env("HT_SKIP_HERDR_TESTS", "1");
        }
        cmd.output().unwrap()
    };
    let out = run(true);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("skipped: no herdr: every_helper_state_is_reachable"),
        "{text}"
    );

    let out = run(false);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success());
    assert!(text.contains("'herdr' not found"), "{text}");
}
