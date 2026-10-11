//! Caller claim derivation from the process environment and ancestry (spec §1, decisions 9, 12, 15).
//!
//! Pure: the environment and the process table are injected. The innermost harness wins: the
//! nearest ancestor shaped like claude, codex or hermes, or like a registered adapter that
//! declares a [`CallerEnvShape`], selects the harness and its own session variable; variables an
//! outer harness left in the environment are ignored. Nothing here is
//! adversarially verified (TRUST-POLICY: cooperative, same-user).
use crate::harness::adapter::{CallerEnvShape, CallerSession};
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
    /// A subagent marker is present but unparseable: refused, never read as top-level.
    MalformedSubagentMarker {
        harness: &'static str,
        var: &'static str,
    },
    /// The harness's local evidence cannot tell whether the session is top-level or a child:
    /// refused, never read as top-level.
    UnattributedSession {
        harness: &'static str,
        detail: String,
    },
}

impl EnvRefusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoHarness => "no_harness",
            Self::MissingSession { .. } => "missing_session",
            Self::NoFingerprint { .. } => "no_fingerprint",
            Self::ManagedDaemon { .. } => "managed_daemon",
            Self::MalformedSubagentMarker { .. } => "malformed_subagent_marker",
            Self::UnattributedSession { .. } => "unattributed_session",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NoHarness => {
                "no claude, codex, hermes or other registered agent harness process among this process's ancestors".into()
            }
            Self::MissingSession { harness, var } => format!(
                "{harness} caller has no session id: {var} is not set in this process's environment \
                 ({harness} did not export {var} to this process: restart the agent - a harness \
                 that predates session-keyed bindings must be restarted after the upgrade - or \
                 re-run `herdr-threads setup` if its hooks are stale)"
            ),
            Self::NoFingerprint { harness, detail } => {
                format!("{harness} caller has no liveness fingerprint: {detail}")
            }
            Self::MalformedSubagentMarker { harness, var } => format!(
                "{harness} caller has a malformed subagent marker in {var}: the call is refused \
                 rather than attributed to the top-level session"
            ),
            Self::UnattributedSession { harness, detail } => format!(
                "{harness} caller's session cannot be attributed to a top-level session or a \
                 subagent: {detail}; the call is refused rather than attributed to the top-level session"
            ),
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

/// The registered adapter (other than the three built into this module) whose declared caller
/// shape names this process, with that shape.
fn registered_shape(record: &ProcessRecord) -> Option<(&'static str, CallerEnvShape)> {
    let argv0 = record.argv.first().map_or("", |a| basename(a));
    crate::harness::registry::builtins()
        .registrations()
        .iter()
        .find_map(|registration| {
            let shape = registration.caller_env()?;
            shape
                .process_names
                .contains(&argv0)
                .then_some((registration.metadata().id, shape))
        })
}

/// The declared caller shape of the registered adapter `id`.
fn shape_of(id: &str) -> Option<CallerEnvShape> {
    crate::harness::registry::builtins()
        .registrations()
        .iter()
        .find(|registration| registration.metadata().id == id)
        .and_then(|registration| registration.caller_env())
}

/// Which harness a process looks like: `claude`, `codex`, `hermes`, or the id of a registered
/// adapter whose [`CallerEnvShape`] names the process.
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
    registered_shape(record).map(|(id, _)| id)
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
    // Test builds: a stand-in agent. `HT_TEST_HARNESS=claude|codex|hermes` makes the spawning test
    // process (this one's parent) the harness, so a test that drives the binary with
    // `--cooperative-*` flags need not build a real process tree (see `test_support::isolation::
    // as_agent`). Never compiled into a release binary.
    #[cfg(any(test, feature = "test-support"))]
    if let Some(kind) = env_var(inputs, "HT_TEST_HARNESS") {
        // A hook names its own harness, which may be a synthetic registered adapter.
        let registered = crate::harness::registry::builtins()
            .registrations()
            .iter()
            .find(|r| r.metadata().id == kind && r.caller_env().is_some())
            .map(|r| r.metadata().id);
        let kind: &'static str = match (kind.as_str(), want, registered) {
            ("claude", _, _) => "claude",
            ("codex", _, _) => "codex",
            ("hermes", _, _) => "hermes",
            (_, _, Some(id)) => id,
            (_, Some(_), None) => "stand-in",
            _ => return Err(EnvRefusal::NoHarness),
        };
        // `HT_TEST_HARNESS_PID` names a long-lived stand-in process for this caller's session
        // (`test_support::isolation::as_agent`), so two sessions are two processes.
        let stand_in = env_var(inputs, "HT_TEST_HARNESS_PID")
            .and_then(|pid| pid.trim().parse::<u32>().ok())
            .or(own.parent);
        if (kind == "stand-in" || want.is_none_or(|w| w == kind))
            && let Some(parent) = stand_in.and_then(|pid| inputs.processes.process(pid))
        {
            return Ok((kind, parent));
        }
    }
    let mut seen = vec![own.pid];
    let mut next = own.parent;
    // Test builds: a test that spawns this binary names its own pid as the top of the ancestry
    // it means to simulate, so the real harness above the test runner never counts.
    #[cfg(any(test, feature = "test-support"))]
    let floor: Option<u32> =
        env_var(inputs, "HT_TEST_ANCESTRY_FLOOR_PID").and_then(|pid| pid.trim().parse().ok());
    for _ in 0..MAX_HOPS {
        let Some(pid) = next.filter(|p| *p > 1) else {
            break;
        };
        if seen.contains(&pid) {
            break;
        }
        #[cfg(any(test, feature = "test-support"))]
        if floor == Some(pid) {
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
            // Fail closed: a distinct but unparseable marker is refused, never downgraded to
            // the top-level session.
            let agent = match env_var(inputs, "CODEX_THREAD_ID")
                .filter(|thread| thread != native.as_str())
            {
                Some(thread) => Some(AgentId::parse(thread).map_err(|_| {
                    EnvRefusal::MalformedSubagentMarker {
                        harness: "codex",
                        var: "CODEX_THREAD_ID",
                    }
                })?),
                None => None,
            };
            (Harness::Codex, native, agent)
        }
        "hermes" => (
            Harness::Agent(
                crate::harness::registry::builtins()
                    .agent("hermes")
                    .expect("hermes is a builtin adapter"),
            ),
            session(inputs, "hermes", "HERMES_SESSION_ID")?,
            None,
        ),
        // A registered adapter's declared shape: its session variable, and its local evidence of
        // the session's role (a child names its parent; undecidable fails toward child).
        registered => {
            let shape = shape_of(registered).ok_or(EnvRefusal::NoHarness)?;
            let agent = crate::harness::registry::builtins()
                .agent(registered)
                .map_err(|_| EnvRefusal::NoHarness)?;
            let native = session(inputs, registered, shape.session_var)?;
            let unattributed = |detail: String| EnvRefusal::UnattributedSession {
                harness: registered,
                detail,
            };
            match shape.role.map(|role| role(native.as_str(), inputs.env)) {
                None | Some(CallerSession::TopLevel) => (Harness::Agent(agent), native, None),
                Some(CallerSession::Subagent { parent }) => {
                    let parent = NativeSessionId::parse(parent)
                        .map_err(|_| unattributed("invalid parent session id".into()))?;
                    let child = AgentId::parse(native.as_str().to_owned()).map_err(|_| {
                        unattributed("child session id is not a valid agent id".into())
                    })?;
                    (Harness::Agent(agent), parent, Some(child))
                }
                Some(CallerSession::Unattributable(detail)) => return Err(unattributed(detail)),
            }
        }
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
