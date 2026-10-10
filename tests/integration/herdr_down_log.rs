//! ht-p03.11: an isolated named Herdr stopped while the daemon runs leaves
//! one first-occurrence line per error class plus capped summaries in
//! daemon.log (never the shared server, never a line per backoff step).
use herdr_threads::{
    daemon::paths::{InstancePaths, RuntimeContext},
    test_support::isolated_herdr::IsolatedHerdr,
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
            "{}/hdl-{}",
            herdr_threads::test_support::SHORT_TMP,
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
    fn daemon_log(&self) -> String {
        let context =
            RuntimeContext::explicit(self.state.clone(), self.host.clone(), None).unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        fs::read_to_string(paths.instance_dir.join("daemon.log")).unwrap_or_default()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.cli(&["daemon", "stop"]);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn observation_lines(log: &str) -> Vec<String> {
    log.lines()
        .filter(|line| line.starts_with("lane observation:"))
        .map(str::to_owned)
        .collect()
}

/// The highest `retrying (attempt N, ...)` count `daemon health` reports.
fn max_retry_attempt(health: &str) -> u32 {
    health
        .split("retrying (attempt ")
        .skip(1)
        .filter_map(|rest| {
            rest.split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|digits| digits.parse().ok())
        })
        .max()
        .unwrap_or(0)
}

/// Failures the observation lane must have recorded before the log is
/// judged: the backoff's first steps (100 ms doubling) come within a few
/// seconds of the first one, so a line per backoff step would show as five
/// lines.
const ATTEMPTS: u32 = 5;

/// Kills: no daemon.log line for the observation lane's failures (silent
/// outage) and a line per backoff step (log flood). The window waits for
/// `ATTEMPTS` consecutive failures (a few seconds of backoff), not the 30 s
/// summary window (`LANE_LOG_WINDOW_MS`): the summary's count, window and
/// attempt are pinned with a fake clock by `daemon::lane_log`
/// (`first_occurrence_then_one_summary_per_window`,
/// `injected_failure_in_each_lane_lands_rate_limited`), and they share this
/// limiter and sink. Any summary that does appear must still be capped and
/// carry its count.
#[test]
fn herdr_stopped_logs_once_then_capped_summaries() {
    let Some(herdr) = IsolatedHerdr::new("herdr_stopped_logs_once_then_capped_summaries") else {
        return;
    };
    herdr.start();
    let scratch = Scratch::new(&herdr.socket_path());
    let ensured = scratch.cli(&["daemon", "ensure"]);
    assert!(
        ensured.status.success(),
        "{}",
        String::from_utf8_lossy(&ensured.stderr)
    );
    assert_eq!(
        observation_lines(&scratch.daemon_log()),
        Vec::<String>::new(),
        "no lane error while Herdr is up"
    );

    herdr.stop();
    let stopped = Instant::now();
    let deadline = stopped + Duration::from_secs(60);
    let mut health;
    loop {
        health = String::from_utf8_lossy(&scratch.cli(&["daemon", "health"]).stdout).into_owned();
        if max_retry_attempt(&health) >= ATTEMPTS {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fewer than {ATTEMPTS} failed attempts within 60 s: {health}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let lines = observation_lines(&scratch.daemon_log());
    let elapsed = stopped.elapsed().as_secs();

    let (summaries, firsts): (Vec<_>, Vec<_>) =
        lines.iter().partition(|line| line.contains(" repeated "));
    assert!(
        !firsts.is_empty(),
        "no daemon.log line after {ATTEMPTS} failures ({health}): {lines:?}"
    );
    // One first-occurrence line per distinct error class, however many
    // backoff steps failed.
    let mut classes: Vec<&str> = firsts
        .iter()
        .map(|line| {
            line["lane observation: ".len()..]
                .split(':')
                .next()
                .unwrap()
        })
        .collect();
    let distinct = {
        classes.sort_unstable();
        classes.dedup();
        classes.len()
    };
    assert_eq!(firsts.len(), distinct, "{lines:?}");
    assert!(
        lines.len() < ATTEMPTS as usize,
        "a line per backoff step ({health}): {lines:?}"
    );
    // Capped: at most one summary per 30 s window per class since the stop.
    assert!(
        summaries.len() <= distinct * (elapsed as usize / 30),
        "{} summaries in {elapsed}s: {lines:?}",
        summaries.len()
    );
    for summary in &summaries {
        assert!(
            summary.contains(" times in the last 30s (attempt "),
            "{summary}"
        );
    }
}
