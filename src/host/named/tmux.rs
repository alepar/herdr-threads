//! The tmux wake path of the named host: the per-binding hint and its verification.
//!
//! Owns: the hint `tmux:<socket>:<server_pid>:<pane>` (built from the caller's `TMUX` and
//! `TMUX_PANE`), its parse, and the verification of one hint against one process: `tmux -S
//! <socket> display -p -t <pane> '#{pane_pid} #{pid}'` under a 2 s deadline, `#{pid}` equal to
//! the hint's server pid, and a nearest-ancestor walk from the binding's process to
//! `#{pane_pid}`.
//!
//! Hides: the `tmux` CLI and its output shape from `NamedHost`.
//!
//! Does not: hold hints (the host does, in memory per binding), type into a pane, or move or
//! rebind a seat. A failed check is a `None`, and the caller disables wake for that binding only.
//!
//! Where new behaviour goes: the pane read and typing commands of the wake path go here too, so
//! every `tmux` invocation shares one deadline and one environment rule.

use std::io::Read;
use std::process::{Command as ProcessCommand, Stdio};
use std::time::{Duration, Instant};

use crate::ports::ProcessInfo;
use crate::protocol::authority::LivenessFingerprint;

/// The tmux client's variables: the server socket with its pid, and the caller's pane.
pub const ENV_TMUX: &str = "TMUX";
pub const ENV_TMUX_PANE: &str = "TMUX_PANE";

/// The deadline of one `tmux` invocation (spec: a fresh `ProcessCommand` with a 2 s deadline).
pub const COMMAND_DEADLINE: Duration = Duration::from_secs(2);

/// Longest hint accepted (a socket path is far shorter).
const MAX_HINT_BYTES: usize = 4096;
/// Longest ancestor chain walked.
const MAX_WALK: usize = 64;

/// A caller's claim about the tmux pane it runs in. A claim, not a fact: only [`verify`] makes it
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxHint {
    pub socket: String,
    pub server_pid: u32,
    pub pane: String,
}

impl TmuxHint {
    /// From the values of `TMUX` (`<socket>,<server_pid>,<session>`) and `TMUX_PANE` (`%<n>`).
    /// `None` when either is malformed: a caller outside tmux, or one with a garbled variable,
    /// sends no hint.
    pub fn from_env(tmux: &str, pane: &str) -> Option<Self> {
        let mut parts = tmux.rsplitn(3, ',');
        let (_session, pid, socket) = (parts.next()?, parts.next()?, parts.next()?);
        Self::new(socket, pid.parse().ok()?, pane)
    }

    fn new(socket: &str, server_pid: u32, pane: &str) -> Option<Self> {
        let pane_ok = pane
            .strip_prefix('%')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        (socket.starts_with('/') && server_pid > 0 && pane_ok).then(|| Self {
            socket: socket.to_owned(),
            server_pid,
            pane: pane.to_owned(),
        })
    }

    /// The wire form, `tmux:<socket>:<server_pid>:<pane>`.
    pub fn encode(&self) -> String {
        format!("tmux:{}:{}:{}", self.socket, self.server_pid, self.pane)
    }

    /// The inverse of [`TmuxHint::encode`]; the socket path may itself contain `:`.
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() > MAX_HINT_BYTES {
            return None;
        }
        let mut parts = text.strip_prefix("tmux:")?.rsplitn(3, ':');
        let (pane, pid, socket) = (parts.next()?, parts.next()?, parts.next()?);
        Self::new(socket, pid.parse().ok()?, pane)
    }
}

/// What the pane query answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaneFacts {
    pane_pid: u32,
    server_pid: u32,
}

/// Run `tmux -S <socket> display -p -t <pane> '#{pane_pid} #{pid}'` with the 2 s deadline.
/// `None` on a missing binary, a missing pane or server, a nonzero exit, a timeout or output of
/// another shape.
fn pane_facts(hint: &TmuxHint) -> Option<PaneFacts> {
    let mut command = ProcessCommand::new("tmux");
    command
        .args(["-S", &hint.socket, "display", "-p", "-t", &hint.pane])
        .arg("#{pane_pid} #{pid}")
        // `-S` names the server; an inherited client variable must not redirect the command.
        .env_remove(ENV_TMUX)
        .env_remove(ENV_TMUX_PANE)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + COMMAND_DEADLINE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let mut out = String::new();
    child
        .stdout
        .take()?
        .take(256)
        .read_to_string(&mut out)
        .ok()?;
    let mut words = out.split_whitespace();
    let facts = PaneFacts {
        pane_pid: words.next()?.parse().ok()?,
        server_pid: words.next()?.parse().ok()?,
    };
    words.next().is_none().then_some(facts)
}

/// Verify `hint` for the binding whose process is `subject_pid`. On success, returns the
/// fingerprint of the long-lived process the verification anchors the binding on: the pane's own
/// process, or its child on the walk (the harness or the job that launched it). The binding is
/// alive while that process is.
///
/// `None` when the server's pid differs from the hint's, the pane does not exist, or `pane_pid`
/// is not an ancestor of (or equal to) `subject_pid`: a stale `TMUX_PANE`, for instance one
/// inherited into a nested Herdr session.
pub fn verify(
    hint: &TmuxHint,
    subject_pid: u32,
    processes: &dyn ProcessInfo,
) -> Option<LivenessFingerprint> {
    let facts = pane_facts(hint)?;
    if facts.server_pid != hint.server_pid {
        return None;
    }
    let mut seen = Vec::new();
    let mut below = None;
    let mut next = Some(subject_pid);
    while let Some(pid) = next {
        if pid == 0 || seen.contains(&pid) || seen.len() >= MAX_WALK {
            return None;
        }
        seen.push(pid);
        let record = processes.process(pid)?;
        if pid == facts.pane_pid {
            return Some(LivenessFingerprint::of(&below.unwrap_or(record)));
        }
        next = record.parent;
        below = Some(record);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_round_trips_through_the_wire_form() {
        let hint = TmuxHint::from_env("/tmp/tmux-1000/default,4242,0", "%7").unwrap();
        assert_eq!(hint.encode(), "tmux:/tmp/tmux-1000/default:4242:%7");
        assert_eq!(TmuxHint::parse(&hint.encode()), Some(hint));
    }

    // Kills: a socket path with ':' or ',' split at the wrong separator.
    #[test]
    fn socket_with_separators_parses_from_the_right() {
        let hint = TmuxHint::from_env("/tmp/a:b,c/sock,99,3", "%1").unwrap();
        assert_eq!(hint.socket, "/tmp/a:b,c/sock");
        assert_eq!(TmuxHint::parse(&hint.encode()), Some(hint));
    }

    // Kills: a malformed variable still producing a hint the daemon would run `tmux` for.
    #[test]
    fn malformed_values_give_no_hint() {
        for (tmux, pane) in [
            ("/s,1,0", "7"),
            ("/s,1,0", "%"),
            ("/s,1,0", "%x"),
            ("/s,0,0", "%1"),
            ("/s,x,0", "%1"),
            ("s,1,0", "%1"),
            ("/s", "%1"),
            ("", "%1"),
        ] {
            assert_eq!(TmuxHint::from_env(tmux, pane), None, "{tmux} {pane}");
        }
        for text in [
            "",
            "tmux:",
            "tmux:/s:1",
            "tmux:/s:x:%1",
            "herdr:/s:1:%1",
            "tmux:s:1:%1",
        ] {
            assert_eq!(TmuxHint::parse(text), None, "{text}");
        }
        assert_eq!(
            TmuxHint::parse(&format!("tmux:/{}:1:%1", "a".repeat(5000))),
            None
        );
    }
}
