//! Runs a command under a synthetic harness parent process, so the command's ancestry looks like
//! it sits under claude, codex or hermes (real-binary tests of `caller_env`).
//!
//! The parent is a `bash` re-exec'd with a chosen argv0 that stays alive until the child exits
//! (no exec), so the child's ancestry shows it. Spawn the returned command through
//! `test_support::spawn::SpawnOwned` so it is stopped with the test.
use std::path::Path;
use std::process::Command;

/// Outer: `$1` is the argv0, the rest are the parent's marker args, `--`, then the program.
const OUTER: &str = r#"h=$1; shift; exec -a "$h" bash -c "$HT_SHIM_INNER" "$@""#;
/// Inner: drop the marker args up to `--`, then run the program as a child (no exec).
const INNER: &str = r#"while [ "$1" != -- ]; do shift; done; shift; "$@"; exit $?"#;

fn build(
    argv0: &str,
    markers: &[&str],
    claude_pid: bool,
    program: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> Command {
    let inner = if claude_pid {
        format!("export CLAUDE_PID=$$; {INNER}")
    } else {
        INNER.to_string()
    };
    let mut command = crate::test_support::spawn::command("bash");
    command
        .arg("-c")
        .arg(OUTER)
        .arg("ht-shim")
        .arg(argv0)
        .args(markers)
        .arg("--")
        .arg(program)
        .args(args)
        .env("HT_SHIM_INNER", inner)
        .envs(env.iter().copied());
    command
}

/// A `Command` that runs `program args` as a child of a `bash` whose argv0 is `harness`
/// ("claude" | "codex" | "hermes"), with `CLAUDE_PID` set to that parent's pid for "claude".
pub fn under_harness(
    harness: &str,
    program: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> Command {
    build(
        harness,
        &["ht-shim"],
        harness == "claude",
        program,
        args,
        env,
    )
}

/// Same, with the parent's argv = `codex app-server --managed-daemon ...` (managed-daemon refusal tests).
pub fn under_codex_managed_daemon(program: &Path, args: &[&str], env: &[(&str, &str)]) -> Command {
    build(
        "codex",
        &["app-server", "--managed-daemon"],
        false,
        program,
        args,
        env,
    )
}
