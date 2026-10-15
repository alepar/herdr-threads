//! Self-cleaning temp paths for tests. `temp_path(name)` is
//! `std::env::temp_dir().join(name)` plus a promise: the path (a file, a
//! directory tree, and for a SQLite file its `-wal`, `-shm` and `-journal`
//! sidecars) is removed when the test process exits, whether or not the test
//! passed. Fixture semantics are unchanged: the path is returned unused and
//! the test creates whatever it wants there.
//!
//! With `HT_LEAK_RUN_ID` set, each path is also appended to
//! `<ledger dir>/ht-leak-ledger-<id>.log` (ledger dir: `HT_LEAK_LEDGER_DIR`,
//! else `temp_dir()`), which `scripts/check-no-leaked-temp-files` reads to
//! report what a run left behind (a killed or timed-out test never runs its
//! exit hook).
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, Once},
};

const SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

static PATHS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static HOOK: Once = Once::new();

extern "C" fn cleanup_at_exit() {
    let paths = match PATHS.lock() {
        Ok(mut paths) => std::mem::take(&mut *paths),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    };
    for path in paths {
        remove_with_sidecars(&path);
    }
}

/// Remove `path` (file, symlink or directory tree) and its SQLite sidecars;
/// absent paths are fine.
pub fn remove_with_sidecars(path: &Path) {
    remove_one(path);
    for suffix in SIDECARS {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        remove_one(Path::new(&sidecar));
    }
}

fn remove_one(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            let _ = std::fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(path);
        }
        Err(_) => {}
    }
}

/// Register an existing or future path for removal at process exit.
pub fn register(path: &Path) {
    HOOK.call_once(|| {
        // SAFETY: `cleanup_at_exit` is a plain `extern "C" fn()`.
        unsafe { libc::atexit(cleanup_at_exit) };
    });
    PATHS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(path.to_path_buf());
    record_in_ledger(path);
}

fn record_in_ledger(path: &Path) {
    let Ok(id) = std::env::var("HT_LEAK_RUN_ID") else {
        return;
    };
    if id.is_empty() || id.contains('/') {
        return;
    }
    let dir = std::env::var_os("HT_LEAK_LEDGER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let ledger = dir.join(format!("ht-leak-ledger-{id}.log"));
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger)
    {
        // One write call per line: O_APPEND keeps concurrent test processes' lines whole.
        let _ = file.write_all(format!("{}\n", path.display()).as_bytes());
    }
}

/// `std::env::temp_dir().join(name)`, removed (with SQLite sidecars) when the
/// process exits.
pub fn temp_path(name: impl AsRef<Path>) -> PathBuf {
    let path = std::env::temp_dir().join(name);
    register(&path);
    path
}

/// `temp_path` under the canonical (symlink-resolved) temp dir, so paths match
/// what the kernel reports back (macOS `/var` vs `/private/var`).
pub fn temp_path_canonical(name: impl AsRef<Path>) -> PathBuf {
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("canonicalize temp dir");
    let path = base.join(name);
    register(&path);
    path
}
