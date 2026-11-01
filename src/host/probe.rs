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
    let mut stream = UnixStream::connect(socket).map_err(drop)?;
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
