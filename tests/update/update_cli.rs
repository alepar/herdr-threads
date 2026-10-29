//! `herdr-threads update` end to end against fake releases on `file://` URLs: the releases
//! API documents, the archives (`scripts/package-release.sh`) and the real
//! `scripts/install.sh` they carry. Each package's `bin/herdr-threads` is a wrapper that
//! execs this test build, naming itself in `HT_TEST_UPDATE_EXE` so `update` finds the
//! installation; `HT_TEST_UPDATE_IGNORE_EXE_VERSION` lets one build stand for every version.
//! HOME, the state root, the bin directory and TMPDIR are scratch; Herdr is never used
//! (`--no-herdr`), and every daemon a test starts is stopped.
use herdr_threads::{
    test_support::{
        scratch::temp_path,
        spawn::{SpawnOwned, command},
    },
    update::release::{asset_name, platform, sha256_file},
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Output,
};

const REPO: &str = env!("CARGO_MANIFEST_DIR");
const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Env {
    root: PathBuf,
    prefix: PathBuf,
    bin_dir: PathBuf,
    state: PathBuf,
    rel: PathBuf,
    api: PathBuf,
}

impl Env {
    fn new(name: &str) -> Self {
        let root = temp_path(format!("update-{name}-{}", uuid::Uuid::new_v4().simple()));
        for dir in [
            "home", "state", "tmp", "rel", "api/tags", "share", "bin", "wrappers",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(
            root.join("THIRD_PARTY_LICENSES.html"),
            "<html>stub</html>\n",
        )
        .unwrap();
        Self {
            prefix: root.join("share").join("herdr-threads"),
            bin_dir: root.join("bin"),
            state: root
                .join("state")
                .join("herdr")
                .join("plugins")
                .join("herdr-threads"),
            rel: root.join("rel"),
            api: root.join("api"),
            root,
        }
    }

    fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
        let mut command = command(program);
        command
            .env("HOME", self.root.join("home"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CONFIG_HOME", self.root.join("home").join(".config"))
            .env("TMPDIR", self.root.join("tmp"))
            .env(
                "HERDR_THREADS_RELEASE_API",
                format!("file://{}", self.api.display()),
            )
            .env("HT_TEST_UPDATE_IGNORE_EXE_VERSION", "1")
            .env("HERDR_BIN", self.root.join("no-herdr"))
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SESSION")
            .env_remove("HERDR_CONFIG_PATH")
            .env_remove("HERDR_THREADS_RELEASE_URL")
            .env_remove("HERDR_THREADS_VERSION")
            .current_dir(&self.root);
        command
    }

    fn run(&self, command: &mut std::process::Command) -> Output {
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn_owned()
            .expect("spawn")
            .wait_with_output()
            .expect("wait")
    }

    /// A package executable for `version`: a wrapper that execs this build, or one whose
    /// daemon commands fail (a release that cannot start its daemon).
    fn wrapper(&self, version: &str, broken: bool) -> PathBuf {
        let path = self
            .root
            .join("wrappers")
            .join(format!("{version}{}", if broken { "-broken" } else { "" }));
        let guard = if broken {
            "case \"$1\" in --state-dir) [ \"$3\" = internal ] && exit 1 ;; esac\n"
        } else {
            ""
        };
        fs::write(
            &path,
            format!("#!/bin/sh\n{guard}HT_TEST_UPDATE_EXE=\"$0\" exec '{BIN}' \"$@\"\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Publish `version` (archive, SHA256SUMS, API document); `bad_digest` misstates the
    /// archive's API digest.
    fn release(&self, version: &str, broken: bool, bad_digest: bool) {
        let platform = platform().unwrap();
        let (os, arch) = platform.split_once('-').unwrap();
        let tag = format!("v{version}");
        let dir = self.rel.join("download").join(&tag);
        fs::create_dir_all(&dir).unwrap();
        let wrapper = self.wrapper(version, broken);
        let out = self.run(
            self.command(Path::new(REPO).join("scripts/package-release.sh"))
                .arg("--binary")
                .arg(&wrapper)
                .args(["--os", os, "--arch", arch, "--version", version, "--out"])
                .arg(&dir)
                .arg("--third-party")
                .arg(self.root.join("THIRD_PARTY_LICENSES.html")),
        );
        assert!(
            out.status.success(),
            "package: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let asset = asset_name(&platform);
        let hash = sha256_file(&dir.join(&asset)).unwrap();
        fs::write(dir.join("SHA256SUMS"), format!("{hash}  {asset}\n")).unwrap();
        let sums_hash = sha256_file(&dir.join("SHA256SUMS")).unwrap();
        let digest = if bad_digest { "0".repeat(64) } else { hash };
        let document = serde_json::json!({
            "tag_name": tag,
            "draft": false,
            "prerelease": false,
            "assets": [
                {"name": asset, "digest": format!("sha256:{digest}"),
                 "browser_download_url": format!("file://{}", dir.join(&asset).display())},
                {"name": "SHA256SUMS", "digest": format!("sha256:{sums_hash}"),
                 "browser_download_url": format!("file://{}", dir.join("SHA256SUMS").display())},
            ],
        });
        let bytes = serde_json::to_vec(&document).unwrap();
        fs::write(self.api.join("tags").join(&tag), &bytes).unwrap();
        fs::write(self.api.join("latest"), &bytes).unwrap();
    }

    fn install(&self, version: &str) {
        let out = self.run(
            self.command("bash")
                .arg(Path::new(REPO).join("scripts/install.sh"))
                .arg("--release-url")
                .arg(format!("file://{}", self.rel.display()))
                .args([
                    "--version",
                    &format!("v{version}"),
                    "--no-herdr",
                    "--no-setup",
                ])
                .arg("--prefix")
                .arg(&self.prefix)
                .arg("--bin-dir")
                .arg(&self.bin_dir),
        );
        assert!(
            self.marker_version() == version,
            "install {version}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// `herdr-threads update ARGS` through the installed link.
    fn update(&self, args: &[&str]) -> Output {
        self.run(
            self.command(self.bin_dir.join("herdr-threads"))
                .arg("update")
                .args(args),
        )
    }

    fn daemon(&self, verb: &str) -> Output {
        self.run(
            self.command(self.prefix.join("bin/herdr-threads"))
                .arg("--state-dir")
                .arg(&self.state)
                .args(["daemon", verb]),
        )
    }

    fn create_store(&self) {
        let node = self.state.join("node");
        fs::create_dir_all(&node).unwrap();
        fs::set_permissions(&self.state, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
        let conn = rusqlite::Connection::open(node.join("herdr-threads.db")).unwrap();
        herdr_threads::store::schema::initialize(
            &conn,
            || herdr_threads::protocol::time::UtcMillis(1),
            &herdr_threads::test_support::test_node_seed(),
        )
        .unwrap();
    }

    fn marker_version(&self) -> String {
        fs::read_to_string(self.prefix.join(".herdr-threads-install"))
            .unwrap_or_default()
            .lines()
            .find_map(|line| line.strip_prefix("version=").map(str::to_owned))
            .unwrap_or_default()
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        PathBuf::from(format!("{}{suffix}", self.prefix.display()))
    }

    fn daemon_running(&self) -> bool {
        fs::read(self.state.join("node/endpoint.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value["pid"].as_i64())
            .is_some_and(|pid| unsafe { libc::kill(pid as i32, 0) } == 0)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if self.prefix.join("bin/herdr-threads").exists() {
            let _ = self.daemon("stop");
        }
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn update_installs_verifies_rotates_refuses_and_resumes() {
    let env = Env::new("flow");
    env.release("0.1.0", false, false);
    env.install("0.1.0");
    // A store and no running daemon: the judge opens it at the target schema, no daemon.
    env.create_store();

    env.release("0.1.1", false, false);
    let dry = env.update(&["--dry-run"]);
    assert!(dry.status.success(), "{}", text(&dry));
    assert!(
        text(&dry).contains("would update 0.1.0 -> 0.1.1"),
        "{}",
        text(&dry)
    );
    assert!(text(&dry).contains("dry run; nothing changed"));
    assert_eq!(env.marker_version(), "0.1.0");
    assert!(!env.sibling(".prev").exists(), "dry run keeps nothing");

    let out = env.update(&[]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains("updated 0.1.0 -> 0.1.1"),
        "{}",
        text(&out)
    );
    assert_eq!(env.marker_version(), "0.1.1");
    assert_eq!(
        fs::read_to_string(env.sibling(".prev/VERSION"))
            .unwrap()
            .trim(),
        "0.1.0"
    );
    assert!(!env.sibling(".update.json").exists(), "journal removed");
    assert!(!env.daemon_running(), "the judge starts no daemon");
    let log = fs::read_to_string(env.sibling(".prev/update.log")).unwrap();
    assert!(
        log.contains("--no-herdr"),
        "the recorded opt-out is replayed: {log}"
    );
    let check: serde_json::Value =
        serde_json::from_slice(&fs::read(env.state.join("node/update-check.json")).unwrap())
            .unwrap();
    assert_eq!(check["latest"], "0.1.1");

    // A second update rotates P.prev.
    env.release("0.1.2", false, false);
    let out = env.update(&[]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(env.marker_version(), "0.1.2");
    assert_eq!(
        fs::read_to_string(env.sibling(".prev/VERSION"))
            .unwrap()
            .trim(),
        "0.1.1"
    );

    let out = env.update(&[]);
    assert!(out.status.success());
    assert!(text(&out).contains("up to date: 0.1.2"), "{}", text(&out));

    let out = env.update(&["--version", "v0.1.0"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{:?} {}",
        out.status,
        text(&out)
    );
    assert!(text(&out).contains("downgrade refused"), "{}", text(&out));

    // A misstated digest refuses before anything changes.
    env.release("0.1.3", false, true);
    let out = env.update(&[]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("checksum mismatch"), "{}", text(&out));
    assert_eq!(env.marker_version(), "0.1.2");

    // A release whose daemon cannot start is rolled back (the schema is unchanged).
    env.release("0.1.4", true, false);
    let out = env.update(&[]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        text(&out).contains("rolled back to 0.1.2"),
        "{}",
        text(&out)
    );
    assert_eq!(env.marker_version(), "0.1.2");
    assert!(!env.sibling(".update.json").exists());

    // An interrupted run is finished first; --dry-run only reports it.
    let journal = serde_json::json!({
        "target": "0.1.2", "previous": "0.1.1",
        "work": env.root.join("gone"), "flags": ["--no-herdr"],
        "states": [{"dir": env.state, "user_version": null, "daemon": "none",
                    "pid": null, "exe": null}],
    });
    fs::write(
        env.sibling(".update.json"),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
    let out = env.update(&["--dry-run"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{:?} {}",
        out.status,
        text(&out)
    );
    assert!(
        text(&out).contains("interrupted update to 0.1.2"),
        "{}",
        text(&out)
    );
    let out = env.update(&[]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("finishing an interrupted update 0.1.1 -> 0.1.2"));
    assert!(!env.sibling(".update.json").exists());
}

#[test]
fn update_waits_for_a_held_lock_then_refuses() {
    let env = Env::new("lock");
    env.release("0.1.0", false, false);
    env.install("0.1.0");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(env.sibling(".update.lock"))
        .unwrap();
    use std::os::unix::io::AsRawFd;
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    let out = env.run(
        env.command(env.bin_dir.join("herdr-threads"))
            .args(["update", "--dry-run"])
            .env("HT_TEST_UPDATE_LOCK_WAIT_MS", "300"),
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "{:?} {}",
        out.status,
        text(&out)
    );
    assert!(
        text(&out).contains("an earlier update is still running"),
        "{}",
        text(&out)
    );
}

#[test]
fn update_refuses_an_executable_outside_an_installation() {
    let env = Env::new("unmanaged");
    let out = env.run(env.command(BIN).arg("update"));
    assert_eq!(
        out.status.code(),
        Some(1),
        "{:?} {}",
        out.status,
        text(&out)
    );
    assert!(
        text(&out).contains("not an installer-managed install"),
        "{}",
        text(&out)
    );
}

#[test]
fn dry_run_names_a_foreign_daemon_it_would_stop() {
    let env = Env::new("foreign");
    env.release("0.1.0", false, false);
    env.install("0.1.0");
    env.create_store();
    env.release("0.1.1", false, false);
    // A live process that looks like a daemon run from elsewhere (a dev checkout).
    let mut foreign = env
        .command("bash")
        .arg("-c")
        .arg(format!(
            "exec -a '/elsewhere/herdr-threads daemon run --state-dir {}' sleep 60",
            env.state.display()
        ))
        .spawn_owned()
        .unwrap();
    let pid = foreign.id();
    fs::write(
        env.state.join("node/endpoint.json"),
        format!("{{\"software_version\":\"0.1.0\",\"pid\":{pid}}}"),
    )
    .unwrap();
    let mut out = env.update(&["--dry-run"]);
    for _ in 0..50 {
        if text(&out).contains("would stop foreign daemon") {
            break;
        }
        // `exec -a` may not have replaced the shell yet.
        std::thread::sleep(std::time::Duration::from_millis(100));
        out = env.update(&["--dry-run"]);
    }
    foreign.stop();
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains(&format!("would stop foreign daemon pid {pid}")),
        "{}",
        text(&out)
    );
    assert!(
        text(&out).contains("/elsewhere/herdr-threads"),
        "{}",
        text(&out)
    );
}

#[test]
fn the_notice_follows_the_pending_hint_gate_once_per_caller() {
    let env = Env::new("notice");
    let node = env.state.join("node");
    fs::create_dir_all(&node).unwrap();
    fs::set_permissions(&env.state, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(node.join("update-check.json"), br#"{"latest":"999.0.0"}"#).unwrap();
    let notice = "[herdr-threads] 999.0.0 available: herdr-threads update";
    let run = |pane: &str, args: &[&str]| {
        let mut command = env.command(BIN);
        command
            .env("HERDR_PANE_ID", pane)
            .arg("--state-dir")
            .arg(&env.state)
            .args(args)
            .args(["thread", "list"]);
        String::from_utf8_lossy(&env.run(&mut command).stdout).into_owned()
    };
    assert!(
        run("p1", &["--machine"]).lines().all(|line| line != notice),
        "--machine"
    );
    assert!(
        run("p1", &["--json"]).lines().all(|line| line != notice),
        "--json"
    );
    // A non-terminal stdout is presentation Auto, not Machine: shown, once per caller.
    assert!(run("p1", &[]).lines().any(|line| line == notice), "auto");
    assert!(
        run("p1", &[]).lines().all(|line| line != notice),
        "once per caller"
    );
    assert!(
        run("p2", &[]).lines().any(|line| line == notice),
        "another caller"
    );
    let offline = {
        let mut command = env.command(BIN);
        command
            .env("HERDR_PANE_ID", "p3")
            .env("HERDR_THREADS_OFFLINE", "1")
            .arg("--state-dir")
            .arg(&env.state)
            .args(["thread", "list"]);
        String::from_utf8_lossy(&env.run(&mut command).stdout).into_owned()
    };
    assert!(
        offline.lines().all(|line| line != notice),
        "offline turns it off"
    );
    let _ = env.run(
        env.command(BIN)
            .arg("--state-dir")
            .arg(&env.state)
            .args(["daemon", "stop"]),
    );
}
