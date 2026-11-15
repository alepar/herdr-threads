//! Cooperative owner consistency, checked against the selected actual pi process.
use crate::harness::adapter::{CallerSession, DecodeFailure, HookInput, LocalEvidence};
use crate::ports::ProcessRecord;

pub const SESSION: &str = "HERDR_THREADS_PI_SESSION";
pub const ROLE: &str = "HERDR_THREADS_PI_ROLE";
pub const MODE: &str = "HERDR_THREADS_PI_MODE";
pub const PID: &str = "HERDR_THREADS_PI_PID";

pub fn valid_session(id: &str) -> bool {
    crate::protocol::ids::NativeSessionId::parse(id).is_ok()
}

pub(super) fn caller_session(session: &str, env: &dyn Fn(&str) -> Option<String>) -> CallerSession {
    if valid_session(session)
        && env(ROLE).as_deref() == Some("top_level")
        && env(MODE).as_deref() == Some("tui")
    {
        CallerSession::TopLevel
    } else {
        CallerSession::Unattributable("pi requires a current explicit top_level TUI owner".into())
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Native process title/binary or official npm interpreter entry. Only captured flagless
/// interpreter and `bun run` forms are supported; arbitrary eval/load wrappers are not.
pub(super) fn process_matches(record: &ProcessRecord) -> bool {
    let Some(program) = record.argv.first().map(|p| basename(p)) else {
        return false;
    };
    if program == "pi" {
        return true;
    }
    let rest = &record.argv[1..];
    let script = match (program, rest) {
        ("node" | "nodejs", [script, ..]) if !script.starts_with('-') => script,
        ("bun", [run, script, ..]) if run == "run" => script,
        ("bun", [script, ..]) if !script.starts_with('-') => script,
        _ => return false,
    };
    // `node /bin/pi` is the npm bin symlink; no generic cli.js or node process qualifies.
    basename(script) == "pi"
        || ["@earendil-works/pi-coding-agent"]
            .into_iter()
            .any(|package| {
                ["dist/bundle/cli.js", "dist/cli.js", "dist/bun/cli.js"]
                    .into_iter()
                    .any(|entry| script.ends_with(&format!("/{package}/{entry}")))
            })
}

pub(super) fn validate_process(
    record: &ProcessRecord,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<(), String> {
    let session = env(SESSION).filter(|sid| valid_session(sid));
    let pid = env(PID)
        .filter(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|pid| pid.parse::<u32>().ok())
        .filter(|pid| *pid > 0);
    if process_matches(record)
        && pid == Some(record.pid)
        && session
            .as_deref()
            .is_some_and(|sid| matches!(caller_session(sid, env), CallerSession::TopLevel))
    {
        Ok(())
    } else {
        Err("pi owner markers do not name the innermost current TUI process".into())
    }
}

/// Pure payload/env/fingerprint fold, shared by every lifecycle and current event, including
/// SessionEnd. A mismatch is an error before any binding event can run.
pub(super) fn hook_evidence(
    input: &HookInput,
    env: &dyn Fn(&str) -> Option<String>,
    fingerprint: Result<crate::protocol::authority::LivenessFingerprint, String>,
) -> Result<LocalEvidence, DecodeFailure> {
    let invalid =
        || DecodeFailure::Invalid("pi: hook owner does not match the current TUI process".into());
    let payload: serde_json::Value = serde_json::from_slice(&input.bytes).map_err(|_| invalid())?;
    let fingerprint = fingerprint.map_err(|_| invalid())?;
    for (field, var) in [("session_id", SESSION), ("role", ROLE), ("mode", MODE)] {
        if payload[field].as_str() != env(var).as_deref() {
            return Err(invalid());
        }
    }
    if payload["pid"].as_u64() != Some(u64::from(fingerprint.pid)) {
        return Err(invalid());
    }
    Ok(LocalEvidence::default())
}

pub(super) fn local_evidence(input: &HookInput) -> Result<LocalEvidence, DecodeFailure> {
    let env = |name: &str| std::env::var(name).ok();
    let inputs = crate::harness::caller_env::EnvInputs {
        env: &env,
        self_pid: std::process::id(),
        agent_id: None,
        processes: &crate::host::process_table::HostProcessTable,
    };
    hook_evidence(
        input,
        &env,
        crate::harness::caller_env::harness_fingerprint(&inputs, "pi").map_err(|e| e.message()),
    )
}
