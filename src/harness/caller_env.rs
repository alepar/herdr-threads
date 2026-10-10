//! Caller claim derivation from the process environment and ancestry (spec §1, decisions 9, 12, 15).
//!
//! Pure: the environment and the process table are injected. The innermost harness wins: the
//! nearest ancestor shaped like claude, codex or hermes selects the harness and its own session
//! variable; variables an outer harness left in the environment are ignored. Nothing here is
//! adversarially verified (TRUST-POLICY: cooperative, same-user).
use crate::ports::{ProcessInfo, ProcessRecord};
use crate::protocol::authority::{
    COOPERATIVE_ENV_SESSION_PROVENANCE, COOPERATIVE_HOOK_AGENT_PROVENANCE, CallerRole, Harness,
    LivenessFingerprint,
};
use crate::protocol::ids::{AgentId, NativeSessionId};

/// Ancestry hops examined before giving up (cycle and runaway-chain bound).
const MAX_HOPS: usize = 64;

pub struct EnvInputs<'a> {
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub self_pid: u32,
    /// `--agent-id` from the CLI (`ParsedCli::agent_id`); honoured only when the innermost harness is Claude.
    pub agent_id: Option<AgentId>,
    pub processes: &'a dyn ProcessInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvClaim {
    pub harness: Harness,
    pub native_session: NativeSessionId,
    pub agent_id: Option<AgentId>,
    /// Subagent iff `agent_id.is_some()`.
    pub role: CallerRole,
    /// A subagent shares its parent's fingerprint on purpose: that is why same-pid supersede
    /// (ht-54r.10) keys on role, so a subagent sharing its parent's pid is never a session switch.
    pub fingerprint: LivenessFingerprint,
    /// `COOPERATIVE_HOOK_AGENT_PROVENANCE` for a Claude `--agent-id`, else `COOPERATIVE_ENV_SESSION_PROVENANCE`.
    pub provenance: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvRefusal {
    NoHarness,
    MissingSession {
        harness: &'static str,
        var: &'static str,
    },
    NoFingerprint {
        harness: &'static str,
        detail: String,
    },
    ManagedDaemon {
        pid: u32,
    },
}

impl EnvRefusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoHarness => "no_harness",
            Self::MissingSession { .. } => "missing_session",
            Self::NoFingerprint { .. } => "no_fingerprint",
            Self::ManagedDaemon { .. } => "managed_daemon",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NoHarness => {
                "no claude, codex or hermes process among this process's ancestors".into()
            }
            Self::MissingSession { harness, var } => format!(
                "{harness} caller has no session id: {var} is not set in this process's environment \
                 (herdr-threads attributes agent calls to the harness session; restart the agent \
                 from a harness version that exports {var})"
            ),
            Self::NoFingerprint { harness, detail } => {
                format!("{harness} caller has no liveness fingerprint: {detail}")
            }
            Self::ManagedDaemon { pid } => format!(
                "this command runs under `codex app-server --managed-daemon` (pid {pid}), which has \
                 no pane; keep Codex in its pane (the pane-env block from branch codex-daemon-env) \
                 or run Codex with --no-daemon"
            ),
        }
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn is_codex(record: &ProcessRecord) -> bool {
    let argv0 = record.argv.first().map_or("", |a| basename(a));
    argv0 == "codex"
        || argv0.starts_with("codex-")
        || record
            .argv
            .iter()
            .any(|a| a.ends_with("/codex.js") || a.contains("@openai/codex"))
}

/// A `codex app-server --managed-daemon` process: no pane, so no pane-bound caller can sit under it.
pub fn is_managed_daemon(record: &ProcessRecord) -> bool {
    is_codex(record)
        && record.argv.iter().any(|a| a == "app-server")
        && record.argv.iter().any(|a| a == "--managed-daemon")
}

/// Which harness a process looks like: `claude`, `codex` or `hermes`.
pub fn classify(record: &ProcessRecord, claude_pid: Option<u32>) -> Option<&'static str> {
    let argv0 = record.argv.first().map_or("", |a| basename(a));
    if Some(record.pid) == claude_pid
        || argv0 == "claude"
        || record
            .argv
            .iter()
            .any(|a| a.contains("@anthropic-ai/claude-code"))
    {
        return Some("claude");
    }
    if is_codex(record) {
        return Some("codex");
    }
    // The `.hermes/bin/hermes` launcher execs `python3 -I -c '<script>' "$@"`.
    if argv0 == "hermes"
        || (argv0.starts_with("python")
            && record
                .argv
                .iter()
                .any(|a| a.contains("hermes_cli") || a.contains("/.hermes/")))
    {
        return Some("hermes");
    }
    None
}

fn env_var(inputs: &EnvInputs<'_>, name: &str) -> Option<String> {
    (inputs.env)(name).filter(|v| !v.is_empty())
}

fn claude_pid(inputs: &EnvInputs<'_>) -> Option<u32> {
    env_var(inputs, "CLAUDE_PID")?.trim().parse().ok()
}

/// Nearest ancestor. `want == None`: the innermost harness of any kind. `Some(h)`: the nearest of `h`.
fn walk(
    inputs: &EnvInputs<'_>,
    want: Option<&str>,
) -> Result<(&'static str, ProcessRecord), EnvRefusal> {
    let claude = claude_pid(inputs);
    let own =
        inputs
            .processes
            .process(inputs.self_pid)
            .ok_or_else(|| EnvRefusal::NoFingerprint {
                harness: "unknown",
                detail: format!(
                    "the process table has no record of this process (pid {})",
                    inputs.self_pid
                ),
            })?;
    let mut seen = vec![own.pid];
    let mut next = own.parent;
    for _ in 0..MAX_HOPS {
        let Some(pid) = next.filter(|p| *p > 1) else {
            break;
        };
        if seen.contains(&pid) {
            break;
        }
        seen.push(pid);
        let Some(record) = inputs.processes.process(pid) else {
            break;
        };
        if is_managed_daemon(&record) {
            return Err(EnvRefusal::ManagedDaemon { pid });
        }
        if let Some(kind) = classify(&record, claude)
            && want.is_none_or(|w| w == kind)
        {
            return Ok((kind, record));
        }
        next = record.parent;
    }
    Err(EnvRefusal::NoHarness)
}

fn fingerprint_for(
    inputs: &EnvInputs<'_>,
    kind: &'static str,
    ancestor: &ProcessRecord,
) -> Result<LivenessFingerprint, EnvRefusal> {
    // Claude exports its own pid; prefer that record when it still exists.
    let record = if kind == "claude" {
        claude_pid(inputs)
            .and_then(|pid| inputs.processes.process(pid))
            .unwrap_or_else(|| ancestor.clone())
    } else {
        ancestor.clone()
    };
    if record.pid == 0 {
        return Err(EnvRefusal::NoFingerprint {
            harness: kind,
            detail: "the harness process has pid 0".into(),
        });
    }
    Ok(LivenessFingerprint::of(&record))
}

fn session(
    inputs: &EnvInputs<'_>,
    harness: &'static str,
    var: &'static str,
) -> Result<NativeSessionId, EnvRefusal> {
    env_var(inputs, var)
        .and_then(|v| NativeSessionId::parse(v).ok())
        .ok_or(EnvRefusal::MissingSession { harness, var })
}

pub fn derive(inputs: &EnvInputs<'_>) -> Result<EnvClaim, EnvRefusal> {
    let (kind, ancestor) = walk(inputs, None)?;
    let (harness, native_session, agent_id) = match kind {
        "claude" => (
            Harness::Claude,
            session(inputs, "claude", "CLAUDE_CODE_SESSION_ID")?,
            inputs.agent_id.clone(),
        ),
        "codex" => {
            let native = session(inputs, "codex", "CODEX_SESSION_ID")?;
            let agent = env_var(inputs, "CODEX_THREAD_ID")
                .filter(|thread| thread != native.as_str())
                .and_then(|thread| AgentId::parse(thread).ok());
            (Harness::Codex, native, agent)
        }
        _ => (
            Harness::Agent(
                crate::harness::registry::builtins()
                    .agent("hermes")
                    .expect("hermes is a builtin adapter"),
            ),
            session(inputs, "hermes", "HERMES_SESSION_ID")?,
            None,
        ),
    };
    let fingerprint = fingerprint_for(inputs, kind, &ancestor)?;
    let provenance = if kind == "claude" && agent_id.is_some() {
        COOPERATIVE_HOOK_AGENT_PROVENANCE
    } else {
        COOPERATIVE_ENV_SESSION_PROVENANCE
    };
    Ok(EnvClaim {
        harness,
        native_session,
        role: if agent_id.is_some() {
            CallerRole::Subagent
        } else {
            CallerRole::TopLevel
        },
        agent_id,
        fingerprint,
        provenance,
    })
}

/// Real process: `std::env`, the host process table, `std::process::id()`.
pub fn derive_current(agent_id: Option<AgentId>) -> Result<EnvClaim, EnvRefusal> {
    let table = crate::host::process_table::HostProcessTable;
    let env = |name: &str| std::env::var(name).ok();
    derive(&EnvInputs {
        env: &env,
        self_pid: std::process::id(),
        agent_id,
        processes: &table,
    })
}

/// The nearest ancestor of the given harness (`claude` | `codex` | `hermes`): hooks know their
/// harness and session from the payload and need only the fingerprint.
pub fn harness_fingerprint(
    inputs: &EnvInputs<'_>,
    harness: &'static str,
) -> Result<LivenessFingerprint, EnvRefusal> {
    let (kind, ancestor) = walk(inputs, Some(harness))?;
    fingerprint_for(inputs, kind, &ancestor)
}

#[cfg(test)]
#[path = "../../tests/harness/caller_env.rs"]
mod tests;
