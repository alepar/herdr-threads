//! Test-owner watch for spawned daemons.
//!
//! Tests start `herdr-threads daemon run` (directly, or through `daemon
//! ensure`, whose detached child outlives the CLI that started it). If the
//! test process panics past its guards, times out, or is killed, that daemon
//! is orphaned and keeps running. A test passes
//! [`test_owner_env`](crate::daemon::lifecycle::test_owner_env) to every
//! command that can start a daemon; the variable is inherited through
//! `daemon ensure` (and hooks) into the detached `daemon run`, which then
//! exits once the owning test process is gone. Only `test-support` builds
//! watch, so release binaries ignore the variable.

#[cfg(feature = "test-support")]
use crate::daemon::lifecycle::TEST_OWNER_PID_ENV;

/// How often a watched daemon checks that its owner is still alive.
#[cfg(feature = "test-support")]
pub const POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// When [`TEST_OWNER_PID_ENV`] names a live process, start a thread that
/// exits this process as soon as that owner no longer exists. A missing,
/// malformed or already-dead owner is ignored (the daemon runs unwatched).
#[cfg(feature = "test-support")]
pub fn watch_from_env() {
    let Some(owner) = std::env::var(TEST_OWNER_PID_ENV)
        .ok()
        .and_then(|value| value.parse::<libc::pid_t>().ok())
        .filter(|pid| *pid > 1)
    else {
        return;
    };
    if !alive(owner) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("test-owner-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(POLL);
                if !alive(owner) {
                    eprintln!("herdr-threads daemon: test owner {owner} exited; stopping");
                    std::process::exit(0);
                }
            }
        });
}

#[cfg(feature = "test-support")]
fn alive(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 performs only the existence and permission check.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
