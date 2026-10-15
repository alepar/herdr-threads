//! `herdr-threads peer --stdio`: the remote end of a peer link, a stateless
//! pump between the ssh stdio pair and the local node daemon socket.
//!
//! Owns: the process-level splice (fd 0 to the socket, the socket to fd 1) and
//! the `HTPEER1\n` preamble written before the first spliced byte. Does not:
//! parse a frame, authenticate, keep state, write a file or a log; the daemon
//! sniffs the first frame and owns everything above the byte stream. Plain
//! synchronous std code: no async runtime beyond the single `ensure_running`
//! call.

use crate::{
    app::SystemClock,
    daemon::{lifecycle::ensure_running, paths::RuntimeContext},
    protocol::time::Clock,
    sync::frame::PREAMBLE,
};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Write},
    mem::ManuallyDrop,
    net::Shutdown,
    os::{fd::FromRawFd, unix::net::UnixStream},
    path::PathBuf,
    sync::Arc,
    thread,
};

/// Exit status when no daemon could be started or reached (stdout untouched).
pub const EXIT_DAEMON_UNAVAILABLE: i32 = 3;
/// Exit status for an I/O error while pumping.
pub const EXIT_IO: i32 = 1;

/// Routing flags of a `peer --stdio` invocation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PeerStdioArgs {
    pub state_dir: Option<PathBuf>,
    pub host_endpoint: Option<PathBuf>,
}

/// Recognize `herdr-threads [--state-dir X] [--host-endpoint H] peer --stdio`
/// (`--flag=VALUE` too). Any other shape, including a repeated routing flag
/// with another value, is `None` and falls through to the ordinary CLI.
pub fn parse_peer_stdio_argv(args: &[OsString]) -> Option<PeerStdioArgs> {
    let mut parsed = PeerStdioArgs::default();
    let mut index = 1;
    loop {
        let text = args.get(index)?.to_str()?;
        let (flag, inline) = text
            .split_once('=')
            .map_or((text, None), |(flag, value)| (flag, Some(value)));
        let slot = match flag {
            "--state-dir" => &mut parsed.state_dir,
            "--host-endpoint" => &mut parsed.host_endpoint,
            "peer" if inline.is_none() => {
                let rest = &args[index + 1..];
                return (rest.len() == 1 && rest[0] == "--stdio").then_some(parsed);
            }
            _ => return None,
        };
        let value = match inline {
            Some(value) => PathBuf::from(value),
            None => {
                index += 1;
                PathBuf::from(args.get(index)?)
            }
        };
        if slot.as_ref().is_some_and(|seen| *seen != value) {
            return None;
        }
        *slot = Some(value);
        index += 1;
    }
}

/// Run the pump and return the process exit status. A pump that got as far as
/// the splice never returns an `Ok` stream to the caller: it exits the process
/// itself once the daemon side has closed.
pub fn run_stdio(context: &RuntimeContext) -> i32 {
    let endpoint = match start_daemon(context) {
        Ok(endpoint) => endpoint,
        Err(detail) => {
            eprintln!("herdr-threads peer: daemon unavailable: {detail}");
            return EXIT_DAEMON_UNAVAILABLE;
        }
    };
    let socket = match UnixStream::connect(&endpoint) {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("herdr-threads peer: daemon unavailable: connect: {error}");
            return EXIT_DAEMON_UNAVAILABLE;
        }
    };
    // The process's own descriptors, never closed by this code.
    // SAFETY: fds 0 and 1 are open for the life of the process and nothing
    // else in this process reads fd 0 or writes fd 1 while the pump runs.
    let mut stdout = ManuallyDrop::new(unsafe { File::from_raw_fd(1) });
    let mut stdin = ManuallyDrop::new(unsafe { File::from_raw_fd(0) });
    if stdout
        .write_all(PREAMBLE)
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return EXIT_IO;
    }
    let (mut up_socket, mut down_socket) = match (socket.try_clone(), socket.try_clone()) {
        (Ok(up), Ok(down)) => (up, down),
        _ => return EXIT_IO,
    };
    thread::spawn(move || {
        // Stdin EOF is the end of this side's stream: tell the daemon, then
        // keep draining the socket until it closes.
        match io::copy(&mut *stdin, &mut up_socket) {
            Ok(_) => {
                let _ = up_socket.shutdown(Shutdown::Write);
            }
            Err(_) => std::process::exit(EXIT_IO),
        }
    });
    let down = thread::spawn(move || match io::copy(&mut down_socket, &mut *stdout) {
        Ok(_) => 0,
        Err(_) => EXIT_IO,
    });
    std::process::exit(down.join().unwrap_or(EXIT_IO));
}

/// Ensure the node daemon (store-only is fine; the host, if any, is only an
/// attach hint) within the standard startup budget and return its socket.
fn start_daemon(context: &RuntimeContext) -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    runtime
        .block_on(ensure_running(context, &executable, clock))
        .map(|descriptor| descriptor.endpoint)
        .map_err(|error| one_line(&error.detail))
}

/// The startup detail on one stderr line, capped so a log tail stays short.
fn one_line(detail: &str) -> String {
    let joined = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    match joined.char_indices().nth(400) {
        Some((end, _)) => format!("{}...", &joined[..end]),
        None => joined,
    }
}

// ---- `peer add|forget|list` (ht-5fr.8) ------------------------------------------
//
// The operator commands build a wire `Command::Sync` from their arguments and
// refuse a malformed one before any daemon call; the daemon decides.

use crate::protocol::{commands::Command, results::ApiError, sync_admin::SyncCommand};

fn checked(command: SyncCommand) -> Result<Command, ApiError> {
    command.validate().map_err(ApiError::invalid_request)?;
    Ok(Command::Sync(command))
}

/// `peer add SSH_DEST [--remote-command C]`: operator-gated.
pub fn add(ssh_dest: String, remote_command: Option<String>) -> Result<Command, ApiError> {
    if ssh_dest.starts_with('-') {
        return Err(ApiError::invalid_request(
            "dial target must not start with '-'",
        ));
    }
    checked(SyncCommand::PeerAdd {
        ssh_dest,
        remote_command,
    })
}

/// `peer forget DEST|NODE`: operator-gated; stops dialing and deletes nothing.
pub fn forget(target: String) -> Result<Command, ApiError> {
    checked(SyncCommand::PeerForget { target })
}

/// `peer list`: every dial target, read-only.
pub fn list() -> Command {
    Command::Sync(SyncCommand::PeerList)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_the_bare_and_routed_forms() {
        let bare = parse_peer_stdio_argv(&argv(&["ht", "peer", "--stdio"])).unwrap();
        assert_eq!(bare, PeerStdioArgs::default());
        let routed = parse_peer_stdio_argv(&argv(&[
            "ht",
            "--state-dir",
            "/s",
            "--host-endpoint=/h",
            "peer",
            "--stdio",
        ]))
        .unwrap();
        assert_eq!(routed.state_dir, Some(PathBuf::from("/s")));
        assert_eq!(routed.host_endpoint, Some(PathBuf::from("/h")));
    }

    #[test]
    fn other_shapes_fall_through_to_the_ordinary_cli() {
        for words in [
            &["ht", "peer"][..],
            &["ht", "peer", "--stdio", "extra"],
            &["ht", "peer", "list"],
            &["ht", "daemon", "run"],
            &[
                "ht",
                "--state-dir",
                "/a",
                "--state-dir",
                "/b",
                "peer",
                "--stdio",
            ],
            &["ht", "--state-dir"],
        ] {
            assert_eq!(parse_peer_stdio_argv(&argv(words)), None, "{words:?}");
        }
    }
}
