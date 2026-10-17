//! Human-readable CLI error rendering and the stable exit-status table
//! documented in `commands::exit_status_help`.

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
        | DaemonBootChanged
        | ServiceBusy
        | StoreBusy
        | DeadlineExceeded
        | Cancelled => EXIT_UNAVAILABLE,
        // A sandbox refusing the daemon socket is an environment the build
        // cannot use as configured (status 4), not an unavailable daemon.
        Unsupported
        | UnsupportedHarness
        | MissingHook
        | CallerUnverified
        | TransportDenied
        | SessionClaimRequired
        | LivenessFingerprintRequired
        | ManagedDaemonRefused => EXIT_UNSUPPORTED,
        UnknownOutcome => EXIT_UNKNOWN_OUTCOME,
        // A subagent's accountable command is refused like any other authorization failure.
        Unauthorized | SubagentReadOnly => EXIT_FAILED,
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
    /// The generic CLI uses 3 for unavailable; watch uses it for an explicit
    /// permanent stop. Only dedicated watch paths may return Exit(3).
    pub(super) fn for_watch(self) -> Self {
        if !matches!(self, Self::Exit(_)) && self.exit_code() == EXIT_UNAVAILABLE {
            Self::RetryableWatchStartup(Box::new(self))
        } else {
            self
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Api(error) | Self::Output(OutputError::Api(error)) => api_exit_code(&error.code),
            Self::Io(error) | Self::Output(OutputError::Io(error)) => io_exit_code(error),
            Self::Exit(code) => *code,
            Self::Usage(_) => EXIT_USAGE,
            Self::RetryableWatchStartup(_) => crate::protocol::watch::WATCH_EXIT_RETRY,
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
            Self::RetryableWatchStartup(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for RunError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn watch_startup_unavailable_errors_retry_without_losing_diagnostics() {
        use ErrorCode::*;
        for code in [
            HostUnavailable,
            DaemonVersionMismatch,
            UnknownWireVersion,
            InstanceMismatch,
            DaemonBootChanged,
            ServiceBusy,
            StoreBusy,
            DeadlineExceeded,
            Cancelled,
        ] {
            for error in [
                RunError::Api(ApiError::new(code.clone(), "restart window")),
                RunError::Output(OutputError::Api(ApiError::new(
                    code.clone(),
                    "restart window",
                ))),
            ] {
                assert_eq!(error.exit_code(), EXIT_UNAVAILABLE, "{code:?}");
                let diagnostic = error.to_string();
                let retry = error.for_watch();
                assert_eq!(
                    retry.exit_code(),
                    crate::protocol::watch::WATCH_EXIT_RETRY,
                    "{code:?}"
                );
                assert_eq!(retry.to_string(), diagnostic);
            }
        }
        for kind in [
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::TimedOut,
            io::ErrorKind::WouldBlock,
        ] {
            for error in [
                RunError::Io(io::Error::from(kind)),
                RunError::Output(OutputError::Io(io::Error::from(kind))),
            ] {
                let diagnostic = error.to_string();
                let retry = error.for_watch();
                assert_eq!(retry.exit_code(), crate::protocol::watch::WATCH_EXIT_RETRY);
                assert_eq!(retry.to_string(), diagnostic);
            }
        }
    }

    #[test]
    fn watch_explicit_stop_and_other_exit_codes_are_preserved() {
        for code in 0..=5 {
            assert!(
                matches!(RunError::Exit(code).for_watch(), RunError::Exit(actual) if actual == code)
            );
        }
        let error = RunError::Api(ApiError::invalid_request("bad args")).for_watch();
        assert_eq!(error.exit_code(), EXIT_USAGE);
        assert!(matches!(error, RunError::Api(_)));
    }

    /// An unsafe state root surfacing as a local Io error (for example from
    /// the cooperative path's `prepare_node_dir`) is invalid local
    /// context, status 2. Kills: dropping the typed unsafe-state check from
    /// `io_exit_code` (PermissionDenied would then fall through to status 1).
    #[test]
    fn unsafe_local_state_io_error_exits_with_usage_status() {
        let root = crate::test_support::scratch::temp_path(format!(
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
