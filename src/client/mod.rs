//! Local IPC clients for the elected daemon.
pub mod local;
pub mod service;

/// Classify a failed Unix-socket connect (root §B3 D3). A refused connect or
/// a missing socket path is a server that is not running (a stale socket
/// whose owner died refuses); a timeout is transient; a permission denial is
/// the sandbox. Consumed by the daemon client, the Herdr host connect path
/// and ht-p03.34 (follow).
pub fn classify_connect_error(
    error: &std::io::Error,
) -> (crate::protocol::results::ErrorClass, &'static str) {
    use crate::protocol::results::ErrorClass;
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::ConnectionRefused | ErrorKind::NotFound => {
            (ErrorClass::Unavailable, "server not running")
        }
        ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted => {
            (ErrorClass::Transient, CONNECT_TIMED_OUT)
        }
        ErrorKind::PermissionDenied => (ErrorClass::Unavailable, "socket refused by sandbox"),
        _ => (ErrorClass::Unavailable, "connection failed"),
    }
}

/// The detail every client gives a connect that timed out. Both producers
/// (the daemon client's own timeout and [`connect_error`] for an OS-level
/// timeout) end with it, which is how [`error_class`] recognizes a slow connect.
pub(crate) const CONNECT_TIMED_OUT: &str = "connect timed out";

/// The failure class of a client error, with a slow connect recognized: the
/// code of a timed-out connect is `host_unavailable` (nothing was sent), whose
/// default class is `Unavailable`, but the daemon is merely slow, so it is
/// `Transient` (root §B10 D3). Everything else keeps the code's default class.
pub fn error_class(
    error: &crate::protocol::results::ApiError,
) -> Option<crate::protocol::results::ErrorClass> {
    use crate::protocol::results::{ErrorClass, ErrorCode};
    if error.code == ErrorCode::HostUnavailable && error.detail.ends_with(CONNECT_TIMED_OUT) {
        return Some(ErrorClass::Transient);
    }
    error.class()
}

/// Map a failed Unix-socket connect to the client's error. A permission
/// denial (EPERM/EACCES, e.g. a Codex seatbelt sandbox refusing the socket)
/// is `transport_denied` with the remedy, never `host_unavailable`: the
/// daemon may be healthy and `daemon ensure` cannot help.
pub(crate) fn connect_error(
    error: &std::io::Error,
    path: &std::path::Path,
) -> crate::protocol::results::ApiError {
    use crate::protocol::results::{ApiError, ErrorCode};
    let (code, detail) = if error.kind() == std::io::ErrorKind::PermissionDenied {
        (
            ErrorCode::TransportDenied,
            format!(
                "daemon socket not reachable from this sandbox (permission denied): {}; \
                 use approved outside-sandbox execution for this CLI command; in Codex, \
                 request command approval or use a preapproved CLI-only rule (see docs/install.md)",
                path.display()
            ),
        )
    } else {
        (
            ErrorCode::HostUnavailable,
            format!(
                "daemon connection unavailable: {}",
                classify_connect_error(error).1
            ),
        )
    };
    ApiError::new(code, detail)
}

#[cfg(test)]
mod tests {
    use super::connect_error;
    use crate::{
        app::SystemClock,
        cli::exit::{EXIT_UNAVAILABLE, EXIT_UNSUPPORTED, api_exit_code},
        protocol::{
            commands::Command,
            results::ErrorCode,
            time::{CallBudget, Cancellation, Clock, MonoInstant},
        },
    };
    use std::{io, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

    /// P2 (Codex demo 2): the seatbelt sandbox refuses `connect()` with
    /// EPERM; a permission denial must not read as a stopped daemon.
    /// Kills: mapping every connect error to host_unavailable.
    #[test]
    fn permission_denied_connect_is_transport_denied_with_remedy() {
        let path = Path::new("/private/tmp/herdr-threads-501/0123456789abcdef.sock");
        for errno in [1, 13] {
            // EPERM (sandbox) and EACCES (filesystem permission).
            let error = connect_error(&io::Error::from_raw_os_error(errno), path);
            assert_eq!(error.code, ErrorCode::TransportDenied, "errno {errno}");
            assert!(error.detail.contains("permission denied"));
            assert!(error.detail.contains(&path.display().to_string()));
            assert!(error.detail.contains("approved outside-sandbox"));
            assert!(!error.detail.contains("setup codex"));
            assert_eq!(api_exit_code(&error.code), EXIT_UNSUPPORTED);
        }
        for kind in [io::ErrorKind::NotFound, io::ErrorKind::ConnectionRefused] {
            let error = connect_error(&io::Error::from(kind), path);
            assert_eq!(error.code, ErrorCode::HostUnavailable);
            assert_eq!(api_exit_code(&error.code), EXIT_UNAVAILABLE);
        }
    }

    /// End to end through the real client: a socket the caller may not
    /// write to (EACCES) yields transport_denied, a missing one
    /// host_unavailable.
    #[test]
    fn local_client_reports_denied_socket_distinctly() {
        let dir = std::env::temp_dir().join(format!(
            "htcd-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("d.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let budget = || CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 2_000),
            cancellation: Cancellation::default(),
        };
        let client = super::local::LocalSocketClient::new(
            path.clone(),
            Arc::clone(&clock),
            uuid::Uuid::new_v4(),
            None,
        );
        let denied =
            crate::ports::LocalClient::call(&client, Command::Health, &budget()).unwrap_err();
        assert_eq!(denied.code, ErrorCode::TransportDenied, "{denied:?}");
        let missing = super::local::LocalSocketClient::new(
            dir.join("absent.sock"),
            Arc::clone(&clock),
            uuid::Uuid::new_v4(),
            None,
        );
        let absent =
            crate::ports::LocalClient::call(&missing, Command::Health, &budget()).unwrap_err();
        assert_eq!(absent.code, ErrorCode::HostUnavailable);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
