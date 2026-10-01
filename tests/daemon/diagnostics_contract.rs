use super::*;
use crate::daemon::ownership::OwnerLock;
use crate::daemon::paths::{InstancePaths, RuntimeContext};
use crate::daemon::{control, lifecycle};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::{Arc, Mutex};

struct LockFixture {
    root: std::path::PathBuf,
    paths: InstancePaths,
}

impl Drop for LockFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn elected_paths() -> LockFixture {
    let root = std::env::temp_dir().join(format!("herdr-diagnostics-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let context = RuntimeContext::explicit(root.clone(), root.join("host.sock"), None).unwrap();
    LockFixture {
        root,
        paths: InstancePaths::resolve(&context).unwrap(),
    }
}

fn elected_lock() -> (LockFixture, OwnerLock) {
    let fixture = elected_paths();
    let lock = OwnerLock::acquire(&fixture.paths).unwrap();
    (fixture, lock)
}

#[test]
fn only_elected_owner_lock_can_install_sink_and_retains_it_through_close() {
    let (fixture, lock) = elected_lock();
    let events = Events::default();
    let listener = lock.bind_socket().unwrap();
    assert_eq!(
        OwnerLock::acquire(&fixture.paths).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    let sink = || LockCheckingSink {
        events: events.clone(),
        paths: fixture.paths.clone(),
    };
    let loser: Option<lifecycle::OwnerSession<LockCheckingSink>> =
        lifecycle::start_after_election(None, sink(), BufferLimits::new(16, 8).unwrap()).unwrap();
    assert!(loser.is_none());
    assert!(events.snapshot().is_empty());
    let owner: lifecycle::OwnerSession<LockCheckingSink> =
        lifecycle::start_after_election(Some(lock), sink(), BufferLimits::new(16, 8).unwrap())
            .unwrap()
            .unwrap();
    assert_eq!(
        OwnerLock::acquire(&fixture.paths).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    control::drain_and_close(owner).unwrap();
    assert_eq!(
        events.snapshot(),
        ["install_with_lock", "drain", "close_with_lock"]
    );
    assert_eq!(
        OwnerLock::acquire(&fixture.paths).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(listener);
    assert!(OwnerLock::acquire(&fixture.paths).is_ok());
}

struct LockCheckingSink {
    events: Events,
    paths: InstancePaths,
}

impl DiagnosticSink for LockCheckingSink {
    fn install(&mut self) -> io::Result<()> {
        assert_eq!(
            OwnerLock::acquire(&self.paths).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        self.events.push("install_with_lock");
        Ok(())
    }
    fn write(&mut self, _: DiagnosticSource, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
    fn drain(&mut self) -> io::Result<()> {
        self.events.push("drain");
        Ok(())
    }
    fn close(&mut self) -> io::Result<()> {
        assert_eq!(
            OwnerLock::acquire(&self.paths).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        self.events.push("close_with_lock");
        Ok(())
    }
}

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<String>>>);
impl Events {
    fn push(&self, event: impl Into<String>) {
        self.0.lock().unwrap().push(event.into());
    }
    fn snapshot(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

struct RecordingSink(Events);
impl DiagnosticSink for RecordingSink {
    fn install(&mut self) -> io::Result<()> {
        self.0.push("install");
        Ok(())
    }
    fn write(&mut self, source: DiagnosticSource, fragment: &[u8]) -> io::Result<()> {
        self.0
            .push(format!("{source:?}:{}", String::from_utf8_lossy(fragment)));
        Ok(())
    }
    fn drain(&mut self) -> io::Result<()> {
        self.0.push("drain");
        Ok(())
    }
    fn close(&mut self) -> io::Result<()> {
        self.0.push("close");
        Ok(())
    }
}

#[test]
fn elected_child_owns_sink_after_ensure_parent_exits_and_drains_before_release() {
    let (_fixture, lock) = elected_lock();
    let events = Events::default();
    events.push("elected_owner");
    let parent = Arc::new(());
    let parent_ref = Arc::downgrade(&parent);
    let mut child = lifecycle::start_after_election(
        Some(lock),
        RecordingSink(events.clone()),
        BufferLimits::new(32, 16).unwrap(),
    )
    .unwrap()
    .unwrap();
    drop(parent);
    assert!(parent_ref.upgrade().is_none());
    child.emit(DiagnosticSource::Daemon, b"boot").unwrap();
    child.flush().unwrap();
    assert!(events.snapshot().iter().any(|event| event == "Daemon:boot"));
    events.push("run_child");
    child.stop_admission();
    events.push("stop_admission");
    control::drain_and_close(child).unwrap();
    assert_eq!(
        events.snapshot(),
        [
            "elected_owner",
            "install",
            "Daemon:boot",
            "drain",
            "run_child",
            "stop_admission",
            "drain",
            "close"
        ]
    );
}

#[test]
fn losing_starter_never_installs_and_oversized_output_stays_bounded() {
    let (_fixture, lock) = elected_lock();
    let events = Events::default();
    let limits = BufferLimits::new(6, 4).unwrap();
    let loser: Option<lifecycle::OwnerSession<RecordingSink>> =
        lifecycle::start_after_election(None, RecordingSink(events.clone()), limits).unwrap();
    assert!(loser.is_none());
    assert!(events.snapshot().is_empty());

    let mut owner =
        lifecycle::start_after_election(Some(lock), RecordingSink(events.clone()), limits)
            .unwrap()
            .unwrap();
    owner.emit(DiagnosticSource::Stdout, b"123456789").unwrap();
    owner.emit(DiagnosticSource::Stderr, b"abcd").unwrap();
    assert_eq!(owner.buffered_bytes(), 4);
    assert_eq!(owner.dropped_bytes(), 9);
    control::drain_and_close(owner).unwrap();
    assert_eq!(
        events.snapshot(),
        ["install", "Stderr:abcd", "drain", "close"]
    );
}

#[test]
fn child_stdout_stderr_reach_owned_sink_after_ensure_fixture_is_gone() {
    let (_fixture, lock) = elected_lock();
    let events = Events::default();
    let parent = Arc::new(());
    let parent_ref = Arc::downgrade(&parent);
    let mut owner = lifecycle::start_after_election(
        Some(lock),
        RecordingSink(events.clone()),
        BufferLimits::new(16, 4).unwrap(),
    )
    .unwrap()
    .unwrap();
    drop(parent);
    assert!(parent_ref.upgrade().is_none());
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("read line; test -z \"$line\"; printf stdouttext; printf stderrtext >&2");
    assert!(
        control::run_child_with_diagnostics(&mut owner, &mut command)
            .unwrap()
            .success()
    );
    let snapshot = events.snapshot();
    let stdout: String = snapshot
        .iter()
        .filter_map(|e| e.strip_prefix("Stdout:"))
        .collect();
    let stderr: String = snapshot
        .iter()
        .filter_map(|e| e.strip_prefix("Stderr:"))
        .collect();
    assert_eq!(stdout, "stdouttext");
    assert_eq!(stderr, "stderrtext");
    assert_eq!(owner.dropped_bytes(), 0);
    assert!(!snapshot.contains(&"close".to_owned()));
    control::drain_and_close(owner).unwrap();
    assert_eq!(events.snapshot().last().unwrap(), "close");
}

struct FailingSink(Events);
impl DiagnosticSink for FailingSink {
    fn install(&mut self) -> io::Result<()> {
        self.0.push("install");
        Ok(())
    }
    fn write(&mut self, _source: DiagnosticSource, _fragment: &[u8]) -> io::Result<()> {
        self.0.push("write_failed");
        Err(io::Error::other("disk full"))
    }
    fn drain(&mut self) -> io::Result<()> {
        self.0.push("drain");
        Ok(())
    }
    fn close(&mut self) -> io::Result<()> {
        self.0.push("close");
        Ok(())
    }
}

#[test]
fn sink_failure_counts_lost_bytes_and_still_closes_before_owner_release() {
    let (_fixture, lock) = elected_lock();
    let events = Events::default();
    let mut owner = lifecycle::start_after_election(
        Some(lock),
        FailingSink(events.clone()),
        BufferLimits::new(16, 8).unwrap(),
    )
    .unwrap()
    .unwrap();
    owner.emit(DiagnosticSource::Daemon, b"first").unwrap();
    owner.emit(DiagnosticSource::Daemon, b"next").unwrap();
    assert!(owner.flush().is_err());
    assert_eq!(owner.dropped_bytes(), 9);
    assert!(owner.emit(DiagnosticSource::Daemon, b"last").is_err());
    assert_eq!(owner.dropped_bytes(), 13);
    control::drain_and_close(owner).unwrap();
    assert_eq!(
        events.snapshot(),
        ["install", "write_failed", "drain", "close"]
    );
}

struct FailOnSecondWrite {
    events: Events,
    writes: usize,
}
impl DiagnosticSink for FailOnSecondWrite {
    fn install(&mut self) -> io::Result<()> {
        self.events.push("install");
        Ok(())
    }
    fn write(&mut self, _source: DiagnosticSource, _fragment: &[u8]) -> io::Result<()> {
        self.writes += 1;
        if self.writes == 2 {
            self.events.push("write_failed");
            Err(io::Error::other("disk full"))
        } else {
            self.events.push("write_ok");
            Ok(())
        }
    }
    fn drain(&mut self) -> io::Result<()> {
        self.events.push("drain");
        Ok(())
    }
    fn close(&mut self) -> io::Result<()> {
        self.events.push("close");
        Ok(())
    }
}

#[test]
fn shutdown_drains_prior_writes_even_if_later_write_fails() {
    let (_fixture, lock) = elected_lock();
    let events = Events::default();
    let sink = FailOnSecondWrite {
        events: events.clone(),
        writes: 0,
    };
    let mut owner =
        lifecycle::start_after_election(Some(lock), sink, BufferLimits::new(16, 8).unwrap())
            .unwrap()
            .unwrap();
    owner.emit(DiagnosticSource::Daemon, b"first").unwrap();
    owner.emit(DiagnosticSource::Daemon, b"next").unwrap();
    assert!(control::drain_and_close(owner).is_err());
    assert_eq!(
        events.snapshot(),
        ["install", "write_ok", "write_failed", "drain", "close"]
    );
}
