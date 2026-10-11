//! Bounded child-process helpers shared by every adapter that gates on an observed
//! installed version or runs a read-only CLI query. Move-only from the Codex adapter.
use crate::protocol::time::{Cancellation, external_bound};
use std::{
    io::Read,
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// The child could not be run, timed out, exited unsuccessfully or left a descendant holding
/// its stdout. Callers map it into their own error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unavailable;

pub const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const VERSION_OUTPUT_LIMIT: u64 = 256;

/// Run `<absolute binary> --version` under one deadline covering process exit
/// and reading stdout to EOF; the process group is killed on expiry. Shared by
/// every adapter that gates on an observed installed version.
#[cfg(test)]
pub(crate) fn version_output(binary: &Path, timeout: Duration) -> Result<Vec<u8>, Unavailable> {
    version_output_by(binary, Instant::now() + external_bound(timeout), None)
}

/// [`version_output`] that also kills the process group once `cancel` fires.
pub(crate) fn version_output_cancellable(
    binary: &Path,
    timeout: Duration,
    cancel: &Cancellation,
) -> Result<Vec<u8>, Unavailable> {
    version_output_by(
        binary,
        Instant::now() + external_bound(timeout),
        Some(cancel),
    )
}

/// Run `<absolute binary> --version` and return its stdout before
/// `deadline`. A process that exits but leaves a descendant holding stdout is
/// `Unavailable`; its process group is killed.
pub(crate) fn version_output_by(
    binary: &Path,
    deadline: Instant,
    cancel: Option<&Cancellation>,
) -> Result<Vec<u8>, Unavailable> {
    bounded_output_by(
        binary,
        &["--version"],
        VERSION_OUTPUT_LIMIT,
        deadline,
        cancel,
    )
}

/// Run `<absolute binary> ARGS...` with no stdin and return at most `limit`
/// bytes of its stdout, only for a successful exit before `timeout`; the
/// process group is killed on expiry. Used for read-only CLI queries.
pub(crate) fn bounded_output(
    binary: &Path,
    args: &[&str],
    limit: u64,
    timeout: Duration,
) -> Result<Vec<u8>, Unavailable> {
    bounded_output_by(
        binary,
        args,
        limit,
        Instant::now() + external_bound(timeout),
        None,
    )
}

pub(crate) fn bounded_output_by(
    binary: &Path,
    args: &[&str],
    limit: u64,
    deadline: Instant,
    cancel: Option<&Cancellation>,
) -> Result<Vec<u8>, Unavailable> {
    let cancelled = || cancel.is_some_and(Cancellation::is_cancelled);
    if !binary.is_absolute() {
        return Err(Unavailable);
    }
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| Unavailable)?;
    let group = child.id() as libc::pid_t;
    let kill_group = || {
        // SAFETY: signals only the fresh process group created for this
        // observation (leader pid == group id).
        unsafe { libc::killpg(group, libc::SIGKILL) };
    };
    let Some(stdout) = child.stdout.take() else {
        kill_group();
        let _ = child.wait();
        return Err(Unavailable);
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline && !cancelled() => {
                std::thread::sleep(Duration::from_millis(5))
            }
            _ => {
                kill_group();
                let _ = child.kill();
                let _ = child.wait();
                return Err(Unavailable);
            }
        }
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let stdout = match receiver.recv_timeout(remaining) {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return Err(Unavailable),
        Err(_) => {
            // A descendant still holds stdout past the deadline.
            kill_group();
            return Err(Unavailable);
        }
    };
    if !status.success() {
        return Err(Unavailable);
    }
    Ok(stdout)
}
