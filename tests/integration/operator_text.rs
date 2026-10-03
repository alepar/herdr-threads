//! ht-p03.46: B3 operator text across the seam. Every remedy line and log path
//! an operator reads (ensure startup failure, the exit-3 table, Health's
//! degraded pointer, doctor's log path) is the text `remedy()` and the
//! `logs::*_path` accessors produce, and no other module spells them.
use herdr_threads::{
    daemon::{
        logs::daemon_log_path,
        paths::{InstancePaths, RuntimeContext},
        remedy::{RemedyContext, remedy},
    },
    protocol::results::ErrorClass,
    test_support::isolated_herdr::{HerdrState, IsolatedHerdr},
};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Scratch {
    root: PathBuf,
    state: PathBuf,
    host: PathBuf,
}
impl Scratch {
    fn new(host: &Path) -> Self {
        let root = PathBuf::from(format!(
            "/private/tmp/hotx-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let state = root.join("st");
        fs::DirBuilder::new().mode(0o700).create(&state).unwrap();
        Self {
            root,
            state,
            host: host.to_path_buf(),
        }
    }
    fn cli(&self, args: &[&str]) -> Output {
        crate::scrubbed_command(BIN)
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.host)
            .args(args)
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"))
            .env("CODEX_HOME", self.root.join("codex-home"))
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .output()
            .unwrap()
    }
    fn paths(&self) -> InstancePaths {
        let context =
            RuntimeContext::explicit(self.state.clone(), self.host.clone(), None).unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        paths.prepare_instance_dir().unwrap();
        paths
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.cli(&["daemon", "stop"]);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Kills: a startup-failure line that is hand-written or names a different
/// file than the attempt's own startup log (the remedy must be exactly
/// `remedy(None, StartupFailure{attempt log})`).
#[test]
fn ensure_startup_failure_prints_remedy_and_attempt_log() {
    let scratch = Scratch::new(&PathBuf::from("/private/tmp/hotx-no-host.sock"));
    let paths = scratch.paths();
    fs::write(
        &paths.database_path,
        b"this is not a sqlite database at all",
    )
    .unwrap();

    let ensure = scratch.cli(&["daemon", "ensure"]);
    let stderr = text(&ensure.stderr);
    assert_eq!(ensure.status.code(), Some(3), "{stderr}");
    let files: Vec<_> = fs::read_dir(paths.instance_dir.join("logs"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let attempt_log = files[0].path();
    let expected = remedy(None, &RemedyContext::StartupFailure { log: attempt_log });
    assert!(
        stderr.lines().any(|line| line.contains(&expected)),
        "no line has {expected:?}: {stderr}"
    );
}

/// Kills: the exit-3 lines in the CLI's exit-status table, or the real
/// exit-3 error, drifting from `remedy(class, Exit3)`.
#[test]
fn exit_3_paths_print_the_class_remedy() {
    let scratch = Scratch::new(&PathBuf::from("/private/tmp/hotx-no-host.sock"));
    let unavailable = scratch.cli(&["inbox", "--seat", "s1"]);
    let stderr = text(&unavailable.stderr);
    assert_eq!(unavailable.status.code(), Some(3), "{stderr}");
    let unavailable_remedy = remedy(Some(ErrorClass::Unavailable), &RemedyContext::Exit3);
    assert!(stderr.contains(&unavailable_remedy), "{stderr}");
    let help = scratch.cli(&["--help"]);
    assert!(help.status.success());
    let help = text(&help.stdout);
    let expected = [
        format!("  3  daemon or host unavailable; {unavailable_remedy}"),
        format!(
            "     (version mismatch: {})",
            remedy(Some(ErrorClass::VersionSkew), &RemedyContext::Exit3)
        ),
    ];
    for line in &expected {
        assert!(
            help.lines().any(|l| l == line),
            "no line equals {line:?}: {help}"
        );
    }
}

/// Kills: doctor printing a log path other than `daemon_log_path`.
#[test]
fn doctor_prints_the_daemon_log_path() {
    let scratch = Scratch::new(&PathBuf::from("/private/tmp/hotx-no-host.sock"));
    let ensured = scratch.cli(&["daemon", "ensure"]);
    assert!(ensured.status.success(), "{}", text(&ensured.stderr));
    let expected = format!(
        "daemon_log: {}",
        daemon_log_path(&scratch.paths()).display()
    );
    let doctor = text(&scratch.cli(&["doctor", "--debug"]).stdout);
    assert!(
        doctor.lines().any(|line| line == expected),
        "no line equals {expected:?}: {doctor}"
    );
}

/// Kills: Health's degraded pointer naming anything but `daemon_log_path`
/// through `remedy()`
/// (isolated Herdr stopped under a running daemon; never the shared server).
#[test]
fn degraded_lane_in_health_prints_see_log() {
    let Some(herdr) = IsolatedHerdr::new("degraded_lane_in_health_prints_see_log") else {
        return;
    };
    herdr.start();
    assert_eq!(herdr.state(), HerdrState::Up);
    let scratch = Scratch::new(&herdr.socket_path());
    let ensured = scratch.cli(&["daemon", "ensure"]);
    assert!(ensured.status.success(), "{}", text(&ensured.stderr));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let doctor = text(&scratch.cli(&["doctor", "--debug"]).stdout);
        if doctor.contains("daemon.host_reachability: ready") {
            break;
        }
        assert!(Instant::now() < deadline, "host never reachable: {doctor}");
        std::thread::sleep(Duration::from_millis(100));
    }
    herdr.stop();
    let log = daemon_log_path(&scratch.paths()).display().to_string();
    let pointers = lane_pointers(&log);
    let shows_pointer = |output: &str| pointers.iter().any(|pointer| output.contains(pointer));
    let deadline = Instant::now() + Duration::from_secs(40);
    let health = loop {
        let health = text(&scratch.cli(&["daemon", "health"]).stdout);
        if shows_pointer(&health) || Instant::now() > deadline {
            break health;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(shows_pointer(&health), "{pointers:?} not in {health}");
}

/// Kills: a hand-written remedy or log-path literal at an operator-text
/// site. Code lines (comments skipped) outside the two owning modules may
/// not carry the literals.
#[test]
fn no_hand_written_remedy_or_log_path_literals_remain() {
    const NEEDLES: [&str; 6] = [
        "daemon.log",
        "startup-",
        "inspect daemon health and logs",
        "matching older executable",
        "degraded: see ",
        "run `herdr-threads daemon ensure`",
    ];
    fn walk(dir: &Path, hits: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, hits);
                continue;
            }
            let name = path.to_string_lossy().into_owned();
            if !name.ends_with(".rs")
                || name.ends_with("src/daemon/logs.rs")
                || name.ends_with("src/daemon/remedy.rs")
            {
                continue;
            }
            for (number, line) in fs::read_to_string(&path).unwrap().lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if NEEDLES.iter().any(|needle| line.contains(needle)) {
                    hits.push(format!("{name}:{}: {}", number + 1, line.trim()));
                }
            }
        }
    }
    let mut hits = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut hits,
    );
    assert!(
        hits.is_empty(),
        "hand-written literals:\n{}",
        hits.join("\n")
    );
}

/// Every `degraded: <remedy>` pointer the failing lane's class could yield (the
/// class is not fixed in advance).
fn lane_pointers(log: &str) -> Vec<String> {
    [
        None,
        Some(ErrorClass::Transient),
        Some(ErrorClass::Unavailable),
        Some(ErrorClass::Corrupt),
    ]
    .into_iter()
    .map(|class| {
        format!(
            "degraded: {}",
            remedy(class, &RemedyContext::LaneDegraded { log: log.into() })
        )
    })
    .collect()
}
