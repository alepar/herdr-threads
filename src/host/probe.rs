//! Kind probe: which kind of server answers on a socket, before any
//! `herdr_hosts` row is written (spec H1b).
//!
//! Responsibility
//! - Owns: one `hello` line, then (for any reply that is not a herdr-core
//!   `core.id`) one Herdr `ping`, each on its own connection, all inside one
//!   deadline.
//! - Hides: the two wire dialects and that a Herdr socket may answer an unknown
//!   method with an error, close, or stay silent.
//! - Must not: write the store, keep a connection, or classify by anything but
//!   the reply (no version gate, no path sniffing).
//! - Role: called by `HostFactory.probe` under `attach_lock` and by `doctor`.
//! - Errors: `HostUnavailable` when the socket does not connect or neither
//!   dialect answers inside the deadline. The call never outlasts `budget`.

use crate::ports::HostKind;
use crate::protocol::results::ApiError;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};

/// The probe deadline production uses.
pub const PROBE_BUDGET: Duration = Duration::from_millis(1_500);
/// A reply line longer than this is not a hello or pong.
const MAX_REPLY_BYTES: u64 = 1024 * 1024;

const HELLO: &str = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"hello\"}\n";
const PING: &str = "{\"id\":\"hello-probe\",\"method\":\"ping\",\"params\":{}}\n";

/// Classifies the server on `socket` as `Hc` (its `hello` reply carries
/// `core.id`) or `Herdr` (its `ping` answers `pong`) within `budget`.
pub fn probe_kind(socket: &Path, budget: Duration) -> Result<HostKind, ApiError> {
    let started = Instant::now();
    // The hello gets half the budget, so a peer that ignores it still leaves
    // the ping its turn and the whole probe stays inside `budget`.
    let hello = exchange(socket, HELLO, started + budget / 2);
    if hello
        .as_ref()
        .is_ok_and(|reply| reply["result"]["core"]["id"].as_str().is_some())
    {
        return Ok(HostKind::Hc);
    }
    match exchange(socket, PING, started + budget) {
        Ok(reply) if reply["result"]["type"] == "pong" => Ok(HostKind::Herdr),
        _ => Err(ApiError::host_unavailable(format!(
            "{} answered neither hello nor ping",
            socket.display()
        ))),
    }
}

/// One request line on a fresh connection, one reply line, all before `until`.
fn exchange(socket: &Path, request: &str, until: Instant) -> Result<Value, ()> {
    let left = |until: Instant| {
        until
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(())
    };
    let mut stream = connect_until(socket, until).map_err(drop)?;
    stream.set_write_timeout(Some(left(until)?)).map_err(drop)?;
    stream.write_all(request.as_bytes()).map_err(drop)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(drop)?).take(MAX_REPLY_BYTES);
    loop {
        // The timeout is re-armed from the remaining budget before each read,
        // so a peer that dribbles bytes cannot stretch the deadline.
        stream.set_read_timeout(Some(left(until)?)).map_err(drop)?;
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(drop)? == 0 || !line.ends_with('\n') {
            return Err(());
        }
        let message: Value = serde_json::from_str(&line).map_err(drop)?;
        // Notification lines (no id) may precede the response.
        if !message.get("id").is_none_or(Value::is_null) {
            return Ok(message);
        }
    }
}

/// Connects to the unix socket at `socket` before `until`. A plain `UnixStream::connect`
/// blocks on a full accept backlog (a wedged server) with no timeout, so this connects
/// nonblocking: Linux answers a full backlog with `EAGAIN` at once, so the call retries every
/// 10 ms until `until`. The returned stream is blocking again and close-on-exec.
pub fn connect_until(socket: &Path, until: Instant) -> std::io::Result<UnixStream> {
    use std::os::{
        fd::{FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    };
    let path = socket.as_os_str().as_bytes();
    // SAFETY: an all-zero `sockaddr_un` is a valid value.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path.is_empty() || path.contains(&0) || path.len() >= addr.sun_path.len() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in addr.sun_path.iter_mut().zip(path) {
        *slot = *byte as libc::c_char;
    }
    // SAFETY: `socket(2)` takes no pointers; the descriptor is owned right after.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `raw` is a fresh descriptor nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    // `SOCK_NONBLOCK`/`SOCK_CLOEXEC` do not exist on macOS: set both with `fcntl`.
    // SAFETY: `fcntl` on a descriptor this function owns takes no pointers.
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL);
        if flags < 0 || libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let fd_flags = libc::fcntl(raw, libc::F_GETFD);
        if fd_flags < 0 || libc::fcntl(raw, libc::F_SETFD, fd_flags | libc::FD_CLOEXEC) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    let len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
    loop {
        // SAFETY: `addr` is a valid `sockaddr_un` of `len` bytes for the call's duration.
        let rc = unsafe { libc::connect(raw, (&raw const addr).cast(), len) };
        if rc == 0 {
            break;
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EISCONN) => break,
            Some(libc::EAGAIN | libc::EINPROGRESS | libc::EALREADY | libc::EINTR) => {
                if Instant::now() >= until {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                std::thread::sleep(
                    Duration::from_millis(10).min(until.saturating_duration_since(Instant::now())),
                );
            }
            _ => return Err(std::io::Error::last_os_error()),
        }
    }
    // SAFETY: as above; the stream reads and writes under the callers' socket timeouts.
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL);
        if flags < 0 || libc::fcntl(raw, libc::F_SETFL, flags & !libc::O_NONBLOCK) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(UnixStream::from(fd))
}
