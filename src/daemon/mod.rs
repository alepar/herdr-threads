//! Elected daemon diagnostics and runtime composition.
pub mod control;
pub mod diagnostics;
pub mod health;
pub mod lifecycle;
pub mod logs;
pub mod ownership;
pub mod paths;
pub mod remedy;
pub mod settings;
pub mod transport;

use crate::daemon::{
    diagnostics::{BufferLimits, DiagnosticSink, DiagnosticSource},
    lifecycle::run_owner_with_factory,
    logs::RotatingLogSink,
    ownership::EndpointDescriptor,
    paths::InstancePaths,
};
use crate::ports::LocalService;
use crate::protocol::time::{Cancellation, Clock};
use std::{
    future::Future,
    io::{self, Read},
    os::fd::{AsRawFd, RawFd},
    os::unix::net::UnixStream,
    sync::{Arc, Mutex, OnceLock},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use uuid::Uuid;

unsafe extern "C" {
    fn dup2(oldfd: RawFd, newfd: RawFd) -> RawFd;
    fn close(fd: RawFd) -> i32;
}

/// A close-on-exec duplicate. The saved stdout/stderr must not leak into the
/// children spawned while output is redirected (ht-zo4: an in-process
/// daemon's saved test-process pipe, inherited as fds 13/14 by every later
/// child, held a test runner's output pipe open after the test exited).
fn duplicate(fd: RawFd) -> io::Result<RawFd> {
    // SAFETY: fcntl(F_DUPFD_CLOEXEC) only creates a new descriptor.
    let result = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

fn replace_fd(from: RawFd, to: RawFd) -> io::Result<()> {
    if unsafe { dup2(from, to) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn close_fd(fd: RawFd) {
    let _ = unsafe { close(fd) };
}

fn lock_log(
    log: &Mutex<RotatingLogSink>,
) -> io::Result<std::sync::MutexGuard<'_, RotatingLogSink>> {
    log.lock()
        .map_err(|_| io::Error::other("diagnostic log lock poisoned"))
}

#[cfg(test)]
fn observe_reader_drain_for_test(deadline: Instant) -> io::Result<()> {
    let Some(root) = std::env::var_os("HERDR_TASK23_DRAIN_PROBE_DIR") else {
        return Ok(());
    };
    let root = std::path::PathBuf::from(root);
    std::fs::write(root.join("drain-entered"), b"")?;
    // This checkpoint only exposes the real reader's drain interval. It is
    // capped inside the already-published production deadline.
    let checkpoint_end = deadline.min(Instant::now() + Duration::from_millis(200));
    loop {
        if Instant::now() >= checkpoint_end {
            let _ = std::fs::write(root.join("probe-timeout"), b"");
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "test reader checkpoint expired",
            ));
        }
        if root.join("release-reader").exists() {
            std::fs::write(root.join("reader-resumed"), b"")?;
            return Ok(());
        }
        thread::sleep(Duration::from_millis(1));
    }
}

/// This sink is installed by lifecycle only after OwnerLock election. The
/// process-wide descriptors belong to the dedicated detached daemon child.
struct ChildOwnedLogSink {
    log: Arc<Mutex<RotatingLogSink>>,
    reader: Option<JoinHandle<ReaderCompletion>>,
    close_deadline: Arc<OnceLock<Instant>>,
    daemon_tail: Vec<u8>,
    saved_stdout: Option<RawFd>,
    saved_stderr: Option<RawFd>,
}

// These bound final application reads even when a descendant keeps the writer
// open. A blocked filesystem write/sync still depends on normal I/O progress.
const FINAL_DRAIN_TIME: Duration = Duration::from_millis(250);
const FINAL_DRAIN_BYTES: usize = 256 * 1024;
const FINAL_DRAIN_READS: usize = 32;
const DAEMON_TAIL_BYTES: usize = 16 * 1024;

struct ReaderCompletion {
    error: Option<io::Error>,
    cutoff_reason: Option<&'static str>,
    bytes_after_close: usize,
    reads_after_close: usize,
}

impl ChildOwnedLogSink {
    fn new(paths: &InstancePaths) -> Self {
        Self {
            log: Arc::new(Mutex::new(RotatingLogSink::new(paths.instance_dir.clone()))),
            reader: None,
            close_deadline: Arc::new(OnceLock::new()),
            daemon_tail: Vec::new(),
            saved_stdout: None,
            saved_stderr: None,
        }
    }

    fn restore_output(&mut self) -> io::Result<()> {
        let mut first_error = None;
        if let Some(saved) = self.saved_stdout.take() {
            if let Err(error) = replace_fd(saved, 1) {
                first_error = Some(error);
            }
            close_fd(saved);
        }
        if let Some(saved) = self.saved_stderr.take() {
            if let Err(error) = replace_fd(saved, 2)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
            close_fd(saved);
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl DiagnosticSink for ChildOwnedLogSink {
    fn install(&mut self) -> io::Result<()> {
        lock_log(&self.log)?.install()?;
        let (mut reader, writer) = UnixStream::pair()?;
        reader.set_read_timeout(Some(Duration::from_millis(100)))?;
        self.saved_stdout = Some(duplicate(1)?);
        self.saved_stderr = Some(duplicate(2)?);
        if let Err(error) =
            replace_fd(writer.as_raw_fd(), 1).and_then(|_| replace_fd(writer.as_raw_fd(), 2))
        {
            let _ = self.restore_output();
            return Err(error);
        }
        let log = Arc::clone(&self.log);
        let close_deadline = Arc::clone(&self.close_deadline);
        let spawned = thread::Builder::new()
            .name("daemon-output-log".into())
            .spawn(move || {
                let mut chunk = [0u8; 16 * 1024];
                let mut first_error = None;
                let mut bytes_after_close = 0usize;
                let mut reads_after_close = 0usize;
                let mut cutoff_reason = None;
                #[cfg(test)]
                let mut drain_observed = false;
                loop {
                    if let Some(deadline) = close_deadline.get() {
                        #[cfg(test)]
                        if !drain_observed {
                            drain_observed = true;
                            if let Err(error) = observe_reader_drain_for_test(*deadline) {
                                first_error.get_or_insert(error);
                            }
                        }
                        cutoff_reason = if Instant::now() >= *deadline {
                            Some("deadline")
                        } else if bytes_after_close >= FINAL_DRAIN_BYTES {
                            Some("byte budget")
                        } else if reads_after_close >= FINAL_DRAIN_READS {
                            Some("read budget")
                        } else {
                            None
                        };
                        if cutoff_reason.is_some() {
                            break;
                        }
                        reads_after_close += 1;
                    }
                    match reader.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(size) => {
                            if first_error.is_none() {
                                let result = lock_log(&log).and_then(|mut sink| {
                                    sink.write(DiagnosticSource::Stdout, &chunk[..size])
                                });
                                if let Err(error) = result {
                                    first_error = Some(error);
                                }
                            }
                            if close_deadline.get().is_some() {
                                bytes_after_close = bytes_after_close.saturating_add(size);
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                            ) =>
                        {
                            // The next iteration checks the absolute close deadline.
                        }
                        Err(error) => {
                            first_error.get_or_insert(error);
                            break;
                        }
                    }
                }
                #[cfg(test)]
                if let Some(root) = std::env::var_os("HERDR_TASK23_DRAIN_PROBE_DIR")
                    && let Err(error) =
                        std::fs::write(std::path::PathBuf::from(root).join("reader-completed"), b"")
                {
                    first_error.get_or_insert(error);
                }
                ReaderCompletion {
                    error: first_error,
                    cutoff_reason,
                    bytes_after_close,
                    reads_after_close,
                }
            });
        match spawned {
            Ok(handle) => self.reader = Some(handle),
            Err(error) => {
                let _ = self.restore_output();
                return Err(error);
            }
        }
        Ok(())
    }

    fn write(&mut self, source: DiagnosticSource, fragment: &[u8]) -> io::Result<()> {
        lock_log(&self.log)?.write(source, fragment)?;
        if source == DiagnosticSource::Daemon {
            if fragment.len() >= DAEMON_TAIL_BYTES {
                self.daemon_tail.clear();
                self.daemon_tail
                    .extend_from_slice(&fragment[fragment.len() - DAEMON_TAIL_BYTES..]);
            } else {
                let excess = self.daemon_tail.len() + fragment.len();
                if excess > DAEMON_TAIL_BYTES {
                    self.daemon_tail.drain(..excess - DAEMON_TAIL_BYTES);
                }
                self.daemon_tail.extend_from_slice(fragment);
            }
        }
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        lock_log(&self.log)?.drain()
    }

    fn close(&mut self) -> io::Result<()> {
        self.close_deadline
            .get_or_init(|| Instant::now() + FINAL_DRAIN_TIME);
        let mut first_error = self.restore_output().err();
        if let Some(reader) = self.reader.take() {
            match reader.join() {
                Ok(completion) => {
                    if first_error.is_none() {
                        first_error = completion.error;
                    }
                    if let Some(reason) = completion.cutoff_reason {
                        let note = format!(
                            "diagnostic raw-output drain cutoff ({reason}) after {} bytes / {} reads; unread output may be lost\n",
                            completion.bytes_after_close, completion.reads_after_close,
                        );
                        if let Err(error) = lock_log(&self.log).and_then(|mut sink| {
                            sink.write(DiagnosticSource::Daemon, note.as_bytes())
                        }) && first_error.is_none()
                        {
                            first_error = Some(error);
                        }
                    }
                }
                Err(_) => {
                    if first_error.is_none() {
                        first_error = Some(io::Error::other("diagnostic reader panicked"));
                    }
                }
            }
        }
        if !self.daemon_tail.is_empty() {
            let replay = b"[final daemon diagnostic replay]\n";
            if let Err(error) = lock_log(&self.log).and_then(|mut sink| {
                sink.write(DiagnosticSource::Daemon, replay)?;
                sink.write(DiagnosticSource::Daemon, &self.daemon_tail)
            }) && first_error.is_none()
            {
                first_error = Some(error);
            }
            self.daemon_tail.clear();
        }
        if let Err(error) = lock_log(&self.log).and_then(|mut sink| sink.close())
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for ChildOwnedLogSink {
    fn drop(&mut self) {
        if self.reader.is_some() || self.saved_stdout.is_some() || self.saved_stderr.is_some() {
            let _ = self.close();
        }
    }
}

/// Run the elected owner with child-owned, two-file rotating diagnostics.
/// Construct the service from the elected instance and bound boot before
/// endpoint publication. Teardown is awaited under the same owner lease.
pub async fn run_elected_with_diagnostics<F, R, D, Fut>(
    paths: &InstancePaths,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
    make_service: F,
    on_ready: R,
    on_drained: D,
) -> io::Result<bool>
where
    F: FnOnce(Uuid, Uuid, Cancellation) -> io::Result<Arc<dyn LocalService>>,
    R: FnOnce(&EndpointDescriptor) -> io::Result<()>,
    D: FnOnce() -> Fut,
    Fut: Future<Output = io::Result<()>>,
{
    run_owner_with_factory(
        paths,
        clock,
        shutdown,
        ChildOwnedLogSink::new(paths),
        BufferLimits::new(64 * 1024, 16 * 1024)?,
        make_service,
        on_ready,
        on_drained,
    )
    .await
}

#[cfg(test)]
#[path = "../../tests/daemon/detached_diagnostics.rs"]
mod tests;
