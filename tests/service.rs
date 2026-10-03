/// The built binary or a helper process with every inherited HERDR_/CLAUDE/CODEX
/// variable removed (ht-p03.24); a test sets the variables it needs after this
/// call. The scrub is part of the `test-support` build; without the feature the
/// suite still compiles and the process is spawned as before.
pub(crate) fn scrubbed_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[cfg_attr(not(feature = "test-support"), allow(unused_mut))]
    let mut command = std::process::Command::new(program);
    #[cfg(feature = "test-support")]
    herdr_threads::test_support::isolation::scrub_env(&mut command);
    command
}

static IN_PROCESS_DAEMON: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// SQLite busy timeout for a fake host's "host I/O is not called under the
/// SQLite writer" probe (`BEGIN IMMEDIATE` from inside the host call).
///
/// Host I/O is synchronous: a writer that the daemon holds across the call
/// cannot be released before the call returns, so such a regression still
/// fails the probe on every run (after this bound instead of at once). A
/// short, unrelated daemon writer (startup, reconciliation, observation
/// admission of another worker) that is merely active at the same instant
/// only delays the probe. A zero timeout turned that benign overlap into a
/// load-dependent `DatabaseBusy` panic.
const HOST_IO_WRITER_PROBE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
#[path = "service/composition.rs"]
mod composition;
#[path = "service/graph_d2.rs"]
mod graph_d2;
#[path = "service/graph_send.rs"]
mod graph_send;
#[path = "service/lanes.rs"]
mod lanes;
#[path = "service/operator.rs"]
mod operator;
#[path = "service/resolution.rs"]
mod resolution;
#[path = "service/retirement_health.rs"]
mod retirement_health;
