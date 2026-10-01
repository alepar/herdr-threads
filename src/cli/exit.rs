//! Human-readable CLI error rendering and the stable exit-status table
//! documented in `commands::EXIT_STATUS_HELP`.

use super::{RunError, output::OutputError};
use crate::protocol::results::{ApiError, ErrorCode};
use std::{fmt, io};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_UNAVAILABLE: i32 = 3;
pub const EXIT_UNSUPPORTED: i32 = 4;
pub const EXIT_UNKNOWN_OUTCOME: i32 = 5;

pub fn api_exit_code(code: &ErrorCode) -> i32 {
    use ErrorCode::*;
    match code {
        InvalidRequest | InvalidCursor | InvalidBudget => EXIT_USAGE,
        HostUnavailable
        | DaemonVersionMismatch
        | UnknownWireVersion
        | InstanceMismatch
        | ServiceBusy
        | StoreBusy
        | DeadlineExceeded
        | Cancelled => EXIT_UNAVAILABLE,
        // A sandbox refusing the daemon socket is an environment the build
        // cannot use as configured (status 4), not an unavailable daemon.
        Unsupported | UnsupportedHarness | MissingHook | CallerUnverified | TransportDenied => {
            EXIT_UNSUPPORTED
        }
        UnknownOutcome => EXIT_UNKNOWN_OUTCOME,
        _ => EXIT_FAILED,
    }
}

fn io_exit_code(error: &io::Error) -> i32 {
    if crate::daemon::paths::is_unsafe_local_state(error) {
        return EXIT_USAGE;
    }
    match error.kind() {
        io::ErrorKind::InvalidInput => EXIT_USAGE,
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::TimedOut
        | io::ErrorKind::WouldBlock => EXIT_UNAVAILABLE,
        _ => EXIT_FAILED,
    }
}

/// Snake-case wire spelling of an error code, e.g. `host_unavailable`.
pub fn code_name(code: &ErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{code:?}"))
}

fn write_api(f: &mut fmt::Formatter<'_>, error: &ApiError) -> fmt::Result {
    let detail = error.detail.trim_end();
    write!(f, "{} ({})", detail, code_name(&error.code))?;
    if let Some(argv) = &error.restart_argv {
        write!(f, "\nrestart: {}", argv.join(" "))?;
    }
    if let Some(bytes) = error.required_minimum_bytes {
        write!(f, "\nrequired minimum bytes: {bytes}")?;
    }
    Ok(())
}

impl RunError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Api(error) | Self::Output(OutputError::Api(error)) => api_exit_code(&error.code),
            Self::Io(error) | Self::Output(OutputError::Io(error)) => io_exit_code(error),
            Self::Exit(code) => *code,
            Self::Usage(_) => EXIT_USAGE,
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Api(error) | Self::Output(OutputError::Api(error)) => write_api(f, error),
            Self::Io(error) | Self::Output(OutputError::Io(error)) => write!(f, "{error}"),
            Self::Exit(code) => write!(f, "exit status {code}"),
            Self::Usage(text) => f.write_str(text.trim_end()),
        }
    }
}

impl std::error::Error for RunError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// An unsafe state root surfacing as a local Io error (for example from
    /// the cooperative path's `prepare_instance_dir`) is invalid local
    /// context, status 2. Kills: dropping the typed unsafe-state check from
    /// `io_exit_code` (PermissionDenied would then fall through to status 1).
    #[test]
    fn unsafe_local_state_io_error_exits_with_usage_status() {
        let root = std::env::temp_dir().join(format!(
            "htux-exit-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o775)).unwrap();
        let error = crate::daemon::paths::check_owned_state_root(&root).unwrap_err();
        std::fs::remove_dir(&root).unwrap();
        assert!(crate::daemon::paths::is_unsafe_local_state(&error));
        assert_eq!(RunError::Io(error).exit_code(), EXIT_USAGE);
        let other = io::Error::new(io::ErrorKind::PermissionDenied, "socket denied");
        assert_eq!(RunError::Io(other).exit_code(), EXIT_FAILED);
    }
}
