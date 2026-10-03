//! Daemon loop inventory (root spec B2, "every daemon loop runs on the Pacer").
//!
//! Every `sleep(`, `interval(`, `recv_timeout`, `park_timeout` or
//! `wait_timeout` under `src/` (except `src/client` and `src/cli`) is either
//! gone (the loop is on a Pacer) or named in [`ALLOWLIST`] with the reason it
//! is not a periodic daemon loop. The test walks the tree with `std::fs`, so a
//! new hit fails it until it is moved onto a Pacer or justified here, and an
//! entry whose line disappeared fails it until the entry is deleted.
use std::path::{Path, PathBuf};

/// `(file, line snippet, reason)`. The snippet is a substring of the hit's
/// trimmed source line.
const ALLOWLIST: &[(&str, &str, &str)] = &[
    (
        "src/host/native.rs",
        "thread::sleep(Duration::from_millis(600))",
        "a unit-test fixture inside native.rs (B5 pane-agent read): the fake \
         Herdr socket answers past the read's limit once; not daemon code",
    ),
    (
        "src/test_support/spawn.rs",
        "std::thread::sleep(Duration::from_millis(20))",
        "test-support only (ht-p03.131): OwnedChild::stop's bounded wait \
         (STOP_GRACE) for a killed test child; not daemon code and not a lane",
    ),
    (
        "src/test_support/owner_watch.rs",
        "std::thread::sleep(POLL)",
        "test-support builds only (ht-6y1): a daemon started by a test polls \
         its owning test process and exits when it is gone; never compiled \
         into a release daemon, and not a lane",
    ),
    (
        "src/daemon/lifecycle.rs",
        ".wait_timeout_while(flag, Duration::from_millis(200)",
        "the starting CLI's one bounded wait for the stderr reader of an \
         already-exited child to drain its pipe before printing the tail; runs \
         in the starter process, never in the daemon",
    ),
    (
        "src/host/transport.rs",
        "tokio::time::sleep(POLL)",
        "per-call budget poll while one host API future is in flight: the \
         deadline is on the injected Clock (a fake in tests) so it cannot be a \
         tokio timer; it ends with the call and is not a lane",
    ),
    (
        "src/host/native.rs",
        "sleep(Duration::from_millis(START_POLL_MILLIS))",
        "bounded confirmation poll inside one native launch call (ends at the \
         call's remaining budget); not a daemon loop",
    ),
    (
        "src/scheduler/mod.rs",
        "recv_timeout(Duration::from_millis(5))",
        "per-attempt lease and cancellation check while one wake attempt runs \
         on a scoped thread; ends with the attempt, which the wake lane's \
         Pacer already schedules",
    ),
    (
        "src/protocol/time.rs",
        ".wait_timeout(guard, remaining)",
        "Cancellation's own blocking wait: a condvar wait to a caller-supplied \
         deadline that the cancel hook wakes at once; the primitive the \
         Pacer's cancellation is built on",
    ),
    (
        "src/harness/codex.rs",
        "sleep(Duration::from_millis(5))",
        "bounded `--version` child poll inside one observation call; the lane \
         that calls it is the admission observer on its Pacer",
    ),
    (
        "src/harness/codex.rs",
        "recv_timeout(remaining)",
        "bounded read of the `--version` child's stdout to the call deadline; \
         not periodic",
    ),
    (
        "src/harness/context.rs",
        "sleep(Duration::from_millis(2))",
        "bounded advisory file-lock retry inside one context operation (ends \
         at `lock_timeout`); not a daemon loop",
    ),
    (
        "src/daemon/control.rs",
        "thread::sleep(pause)",
        "the CLI's wait for a stopping daemon to release its lock, bounded by \
         the stop deadline; runs in the client process, not the daemon",
    ),
    (
        "src/daemon/lifecycle.rs",
        "tokio::time::sleep(",
        "client-side `ensure daemon` start-up poll bounded by the call \
         deadline (spawn, then wait for the owner lock and endpoint); not a \
         daemon loop",
    ),
    (
        "src/daemon/mod.rs",
        "thread::sleep(Duration::from_millis(1))",
        "test-reader checkpoint capped at 200 ms inside a published deadline \
         (failpoint drain probe); test support",
    ),
    (
        "src/service/fair_writer.rs",
        ".wait_timeout(state, Duration::from_millis(10))",
        "bounded admission wait for the writer turn: a condvar re-check while \
         one caller queues, woken by the previous turn's release; not periodic",
    ),
    (
        "src/service/workers.rs",
        ".wait_timeout(state, Duration::from_millis(10))",
        "`BoundedLane::enter`'s bounded admission wait for one request; not a \
         periodic loop",
    ),
    (
        "src/service/pacer.rs",
        ".wait_timeout(st, timeout)",
        "the Pacer itself: the one place a lane blocks",
    ),
    (
        "src/service/pacer.rs",
        "tokio::time::sleep(Duration::from_millis(remaining_ms.max(1)))",
        "the Pacer's async twin: the one timer an async lane waits on",
    ),
    (
        "src/store/mod.rs",
        "sleep(std::time::Duration::from_millis(remaining.min(2)))",
        "writer-lock contention retry for one budgeted request (ends at the \
         call deadline); not a daemon loop",
    ),
    (
        "src/store/connection.rs",
        "sleep(Duration::from_millis(remaining.min(2)))",
        "SQLite busy/progress handler pause inside one budgeted statement \
         (ends at the call deadline); not a daemon loop",
    ),
    (
        "src/test_support/failpoints.rs",
        ".wait_timeout(state, remaining)",
        "test support: failpoint gate wait to a test deadline",
    ),
    (
        "src/test_support/server_completion.rs",
        ".wait_timeout(",
        "test support: server-completion gate wait to a test deadline",
    ),
    (
        "src/test_support/history.rs",
        "sleep(std::time::Duration::from_millis(",
        "test support: history probe polling to a test deadline",
    ),
    (
        "src/test_support/search_barrier.rs",
        ".wait_timeout(",
        "test support: search-barrier gate wait to a test deadline",
    ),
];

/// Files whose loops were converted onto a Pacer or to event-driven waits
/// (transport ht-p03.9.2); none may appear on the allowlist.
const CONVERTED_FILES: &[&str] = &["src/daemon/transport.rs"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

/// `\bsleep\(|interval\(|recv_timeout|park_timeout|wait_timeout`.
fn is_loop_hit(line: &str) -> bool {
    let bare_sleep = line.match_indices("sleep(").any(|(at, _)| {
        line[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    });
    bare_sleep
        || ["interval(", "recv_timeout", "park_timeout", "wait_timeout"]
            .iter()
            .any(|needle| line.contains(needle))
}

/// Every hit as `(file, 1-based line, trimmed source line)`.
fn hits() -> Vec<(String, usize, String)> {
    let mut files = Vec::new();
    rust_files(&root().join("src"), &mut files);
    let mut hits = Vec::new();
    for path in files {
        let file = relative(&path);
        if file.starts_with("src/client/") || file.starts_with("src/cli/") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (index, line) in text.lines().enumerate() {
            if is_loop_hit(line) {
                hits.push((file.clone(), index + 1, line.trim().to_string()));
            }
        }
    }
    hits
}

fn entry_for<'a>(file: &str, line: &str) -> Option<&'a (&'static str, &'static str, &'static str)> {
    ALLOWLIST
        .iter()
        .find(|(entry_file, snippet, _)| *entry_file == file && line.contains(snippet))
}

#[test]
fn every_timed_wait_is_on_a_pacer_or_allowlisted_with_a_reason() {
    let hits = hits();
    assert!(
        !hits.is_empty(),
        "the scan found nothing: the pattern is broken"
    );
    let unlisted: Vec<String> = hits
        .iter()
        .filter(|(file, _, line)| entry_for(file, line).is_none())
        .map(|(file, number, line)| format!("{file}:{number}: {line}"))
        .collect();
    assert!(
        unlisted.is_empty(),
        "move these onto a Pacer or add a justified ALLOWLIST entry:\n{}",
        unlisted.join("\n")
    );
    let stale: Vec<String> = ALLOWLIST
        .iter()
        .filter(|(file, snippet, _)| {
            !hits
                .iter()
                .any(|(hit_file, _, line)| hit_file == file && line.contains(snippet))
        })
        .map(|(file, snippet, _)| format!("{file}: {snippet}"))
        .collect();
    assert!(
        stale.is_empty(),
        "stale ALLOWLIST entries:\n{}",
        stale.join("\n")
    );
    for (file, snippet, reason) in ALLOWLIST {
        assert!(
            reason.split_whitespace().count() >= 4,
            "{file}: {snippet}: a reason is required"
        );
    }
}

#[test]
fn converted_lane_and_transport_loops_are_not_allowlisted() {
    for (file, snippet, _) in ALLOWLIST {
        assert!(
            !CONVERTED_FILES.contains(file),
            "{file} ({snippet}) was converted and must stay off the allowlist"
        );
    }
    // The only workers.rs exceptions sit in `BoundedLane`, ahead of every lane
    // (deadline, wake, observation, retention, admission observer) function.
    let workers = std::fs::read_to_string(root().join("src/service/workers.rs")).unwrap();
    let first_lane = workers
        .lines()
        .position(|line| line.contains("pub fn start_deadline_worker"))
        .expect("the deadline lane fn")
        + 1;
    let bounded_lane_end = workers
        .lines()
        .position(|line| line.contains("impl Drop for LaneGuard"))
        .expect("BoundedLane's guard")
        + 1;
    assert!(bounded_lane_end < first_lane);
    for (file, number, line) in hits() {
        if file == "src/service/workers.rs" {
            assert!(
                number <= bounded_lane_end,
                "workers.rs:{number} is inside a lane fn, not BoundedLane: {line}"
            );
        }
        assert!(
            !CONVERTED_FILES.contains(&file.as_str()),
            "{file}:{number} reintroduced a timed wait: {line}"
        );
    }
}

#[test]
fn epic_rg_finds_no_10_or_20ms_sleeps() {
    let mut files = Vec::new();
    for dir in ["src/daemon", "src/client", "src/service"] {
        rust_files(&root().join(dir), &mut files);
    }
    assert!(files.len() > 10, "the scan found too few files");
    let mut found = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        for (index, line) in text.lines().enumerate() {
            if ["10", "20"]
                .iter()
                .any(|ms| line.contains(&format!("sleep(Duration::from_millis({ms}))")))
            {
                found.push(format!(
                    "{}:{}: {}",
                    relative(&path),
                    index + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        found.is_empty(),
        "fixed 10/20 ms sleeps:\n{}",
        found.join("\n")
    );
}
