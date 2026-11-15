use super::*;
use std::{fs, os::unix::fs::symlink, time::Duration};
struct Fixture {
    root: PathBuf,
    proc: PathBuf,
    database: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = crate::test_support::scratch::temp_path(format!(
            "ht-wake-capture-{}",
            uuid::Uuid::new_v4()
        ));
        let proc = root.join("proc");
        fs::create_dir_all(proc.join("fd")).unwrap();
        fs::write(proc.join("cmdline"), b"codex\0--no-daemon\0").unwrap();
        fs::write(
            proc.join("environ"),
            b"CODEX_HOME=/unused\0OPENAI_API_KEY=SECRET_NOT_TO_RETAIN\0",
        )
        .unwrap();
        let executable = root.join("codex");
        fs::write(&executable, b"fixture executable").unwrap();
        symlink(executable, proc.join("exe")).unwrap();
        let database = root.join("queue_1.sqlite");
        fs::write(&database, b"fixture DB").unwrap();
        let lock = root.join("thread-writer-locks/00000000-0000-0000-0000-000000000001.lock");
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        fs::write(&lock, b"").unwrap();
        symlink(&database, proc.join("fd/1")).unwrap();
        symlink(&database, proc.join("fd/2")).unwrap();
        symlink(lock, proc.join("fd/3")).unwrap();
        Self {
            root,
            proc,
            database,
        }
    }
    fn read(&self) -> Option<Descriptor> {
        files(
            &self.proc,
            "00000000-0000-0000-0000-000000000001",
            Instant::now() + Duration::from_millis(100),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn same_database_multiple_descriptors_are_one_target() {
    let f = Fixture::new();
    let d = f.read().expect("one actual DB plus session lock qualifies");
    assert_eq!(d.database.path, f.database);
    assert!(
        !serde_json::to_string(&d)
            .unwrap()
            .contains("SECRET_NOT_TO_RETAIN")
    );
}
#[test]
fn distinct_queue_databases_refuse_capture() {
    let f = Fixture::new();
    let other = f.root.join("other/queue_1.sqlite");
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    fs::write(&other, b"other").unwrap();
    symlink(other, f.proc.join("fd/4")).unwrap();
    assert!(f.read().is_none());
}
#[test]
fn missing_session_lock_refuses_capture() {
    let f = Fixture::new();
    fs::remove_file(f.proc.join("fd/3")).unwrap();
    assert!(f.read().is_none());
}
#[test]
fn replaced_database_path_refuses_capture() {
    let f = Fixture::new();
    let old = f.root.join("old");
    fs::rename(&f.database, &old).unwrap();
    fs::write(&f.database, b"replacement").unwrap();
    fs::remove_file(f.proc.join("fd/1")).unwrap();
    fs::remove_file(f.proc.join("fd/2")).unwrap();
    symlink(
        format!("{} (deleted)", f.database.display()),
        f.proc.join("fd/1"),
    )
    .unwrap();
    assert!(f.read().is_none());
}

#[test]
fn oversized_environment_refuses_without_retaining_values() {
    let f = Fixture::new();
    fs::write(f.proc.join("environ"), vec![b'x'; 65537]).unwrap();
    assert!(f.read().is_none());
}
#[test]
fn managed_daemon_refuses_capture() {
    let f = Fixture::new();
    fs::write(
        f.proc.join("cmdline"),
        b"codex\0app-server\0--managed-daemon\0",
    )
    .unwrap();
    assert!(f.read().is_none());
}

#[test]
fn deleted_queue_descriptor_alongside_live_database_refuses_capture() {
    let f = Fixture::new();
    symlink(
        f.root.join("old/queue_1.sqlite (deleted)"),
        f.proc.join("fd/4"),
    )
    .unwrap();
    assert!(
        f.read().is_none(),
        "a surviving old queue connection makes the storage scope ambiguous"
    );
}
