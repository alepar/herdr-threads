//! ht-p03.11: the per-attempt startup log sink, ensure's own-attempt tail and
//! pruning. The detached child is a stand-in script, so each test controls
//! exactly what it writes to stderr and how it exits.
use super::*;
use crate::daemon::logs::{create_startup_log, prune_startup_logs, startup_log_path};
use crate::daemon::paths::{InstancePaths, RuntimeContext};
use crate::protocol::time::{Clock, MonoInstant, UtcMillis};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

struct WallClock;
impl Clock for WallClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        use std::sync::OnceLock;
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        MonoInstant(
            START
                .get_or_init(std::time::Instant::now)
                .elapsed()
                .as_millis() as u64,
        )
    }
}

fn scratch() -> (PathBuf, RuntimeContext, InstancePaths) {
    let root = std::env::temp_dir().join(format!("herdr-startlog-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let context = RuntimeContext::explicit(root.clone(), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    (root, context, paths)
}

/// A stand-in daemon: prints `message` to stderr and exits with `code`.
fn fake_daemon(root: &Path, name: &str, message: &str, code: i32) -> PathBuf {
    let path = root.join(name);
    std::fs::write(
        &path,
        format!("#!/bin/sh\necho '{message}' >&2\nexit {code}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn startup_files(paths: &InstancePaths) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(startup_logs_dir(paths)) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    files.sort();
    files
}

async fn ensure_with(context: &RuntimeContext, exe: &Path) -> ApiError {
    ensure_running_with_timeout(context, exe, Arc::new(WallClock), Duration::from_secs(10))
        .await
        .unwrap_err()
}

/// Kills: sending the child's stderr to null (no tail), printing a tail
/// without naming the attempt's file, and printing nothing of the child's
/// own words.
#[tokio::test]
async fn daemon_failing_before_election_prints_its_attempt_tail() {
    let (root, context, paths) = scratch();
    let exe = fake_daemon(&root, "daemon-a", "store startup: corrupt page 7", 2);
    let error = ensure_with(&context, &exe).await;
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    let files = startup_files(&paths);
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with(&format!("startup-{}-", std::process::id())),
        "{name}"
    );
    assert_eq!(
        std::fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        error.detail.contains("store startup: corrupt page 7"),
        "{}",
        error.detail
    );
    assert!(
        error.detail.contains(&files[0].display().to_string()),
        "{}",
        error.detail
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// Kills: no fallback when `<state>/logs` is unusable (the child's error is
/// lost), and a fallback that does not say where the text came from.
#[tokio::test]
async fn unwritable_logs_dir_falls_back_to_piped_stderr() {
    let (root, context, paths) = scratch();
    paths.prepare_instance_dir().unwrap();
    std::fs::write(startup_logs_dir(&paths), b"not a directory").unwrap();
    let exe = fake_daemon(&root, "daemon-a", "store startup: corrupt page 7", 2);
    let error = ensure_with(&context, &exe).await;
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert!(
        error.detail.contains("startup log unavailable"),
        "{}",
        error.detail
    );
    assert!(
        error.detail.contains("showing the child's stderr"),
        "{}",
        error.detail
    );
    assert!(
        error.detail.contains("store startup: corrupt page 7"),
        "{}",
        error.detail
    );
    assert!(error.detail.contains("daemon.log"), "{}", error.detail);
    std::fs::remove_dir_all(root).unwrap();
}

/// Kills: a shared fixed startup path (the later child overwrites the
/// earlier one's output), and a tail read from the newest file in the
/// directory instead of the starter's own.
#[tokio::test]
async fn racing_starters_print_only_their_own_tail() {
    let (root, context, paths) = scratch();
    let a = fake_daemon(&root, "daemon-a", "ONLY-IN-A child failed", 2);
    let b = fake_daemon(&root, "daemon-b", "ONLY-IN-B child failed", 2);
    let (first, second) = tokio::join!(ensure_with(&context, &a), ensure_with(&context, &b));
    assert!(first.detail.contains("ONLY-IN-A"), "{}", first.detail);
    assert!(!first.detail.contains("ONLY-IN-B"), "{}", first.detail);
    assert!(second.detail.contains("ONLY-IN-B"), "{}", second.detail);
    assert!(!second.detail.contains("ONLY-IN-A"), "{}", second.detail);
    // Same process id for both starters: the nonce keeps the files apart.
    assert_eq!(startup_files(&paths).len(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

/// Kills: no pruning, pruning by count only (a 25 h file survives among the
/// newest eight), age only (more than eight survive), and deleting the
/// attempt's own file.
#[test]
fn pruning_keeps_eight_newest_and_drops_older_than_24h() {
    let (root, _context, paths) = scratch();
    paths.prepare_instance_dir().unwrap();
    let attempt = StartAttempt::new();
    let (_file, current) = create_startup_log(&paths, &attempt).unwrap();
    let dir = startup_logs_dir(&paths);
    let now = SystemTime::now();
    let make = |name: &str, age_hours: u64, extra_secs: u64| {
        let path = dir.join(name);
        let file = std::fs::File::create(&path).unwrap();
        file.set_modified(now - Duration::from_secs(age_hours * 3600 + extra_secs))
            .unwrap();
        path
    };
    // 3 older than 24 h, 9 recent (ages 1..9 minutes).
    let old: Vec<_> = (0..3)
        .map(|i| make(&format!("startup-1-old{i}.log"), 25 + i, 0))
        .collect();
    let recent: Vec<_> = (0..9)
        .map(|i| make(&format!("startup-2-new{i}.log"), 0, 60 * (i + 1)))
        .collect();
    let unrelated = dir.join("notes.txt");
    std::fs::write(&unrelated, b"keep").unwrap();

    prune_startup_logs(&dir, &current, now);

    let left: Vec<_> = startup_files(&paths)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "log"))
        .collect();
    assert_eq!(left.len(), 8, "{left:?}");
    assert!(current.exists());
    assert!(old.iter().all(|path| !path.exists()));
    // The seven newest recent files survive; the two oldest recent are gone.
    assert!(recent[..7].iter().all(|path| path.exists()));
    assert!(recent[7..].iter().all(|path| !path.exists()));
    assert!(unrelated.exists());
    assert_eq!(startup_log_path(&paths, &attempt), current);
    std::fs::remove_dir_all(root).unwrap();
}

/// Kills: reusing an existing attempt file (O_EXCL dropped) and a log that is
/// not private.
#[test]
fn startup_log_is_exclusive_and_private() {
    let (root, _context, paths) = scratch();
    paths.prepare_instance_dir().unwrap();
    let attempt = StartAttempt::new();
    let (_file, path) = create_startup_log(&paths, &attempt).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(startup_logs_dir(&paths))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let again = create_startup_log(&paths, &attempt).unwrap_err();
    assert_eq!(again.kind(), io::ErrorKind::AlreadyExists);
    std::fs::remove_dir_all(root).unwrap();
}

/// Kills: a tail that prints the whole file (more than 40 lines) and one that
/// lets terminal escapes through.
#[test]
fn tail_is_bounded_and_defanged() {
    let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let tail = crate::daemon::logs::startup_tail_text(text.as_bytes());
    assert_eq!(tail.lines().count(), 40);
    assert!(tail.ends_with("line 99"));
    assert!(tail.starts_with("line 60"));
    let escaped = crate::daemon::logs::startup_tail_text(b"\x1b[31mred\x07\n");
    assert_eq!(escaped, "?[31mred?");
}
