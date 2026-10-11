//! Caller claim derivation from the process environment and ancestry (spec §1, decisions 9, 12, 15).
//!
//! Pure: the environment and the process table are injected. The innermost harness wins: the
//! nearest ancestor shaped like a registered adapter's declared [`CallerEnvShape`] selects the
//! harness and its own session variable; variables an outer harness left in the environment are
//! ignored. Nothing here is
//! adversarially verified (TRUST-POLICY: cooperative, same-user).
use crate::harness::adapter::{CallerEnvShape, CallerSession};
use crate::harness::registry::builtins;
use crate::ports::{ProcessInfo, ProcessRecord};
use crate::protocol::authority::{
    COOPERATIVE_ENV_SESSION_PROVENANCE, COOPERATIVE_HOOK_AGENT_PROVENANCE, CallerRole, Harness,
    LivenessFingerprint,
};
use crate::protocol::ids::{AgentId, NativeSessionId};

/// Ancestry hops examined before giving up (cycle and runaway-chain bound).
const MAX_HOPS: usize = 64;

/// The caller's environment lookup.
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

pub struct EnvInputs<'a> {
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub self_pid: u32,
    /// `--agent-id` from the CLI (`ParsedCli::agent_id`); honoured only when the innermost harness's binding policy takes a hook-passed agent id.
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
    /// `COOPERATIVE_HOOK_AGENT_PROVENANCE` for a hook-passed `--agent-id`, else `COOPERATIVE_ENV_SESSION_PROVENANCE`.
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
            Self::NoHarness => format!(
                "no {} or other registered agent harness process among this process's ancestors",
                caller_harness_names()
            ),
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
            Self::ManagedDaemon { pid } => builtins()
                .caller_shapes()
                .find_map(|(_, shape)| shape.paneless)
                .map_or_else(
                    || {
                        format!(
                            "this command runs under a paneless harness daemon (pid {pid}); run \
                             the agent in its pane"
                        )
                    },
                    |paneless| (paneless.refusal)(*pid),
                ),
        }
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The ids of the registered harnesses that declare a caller shape, sorted and joined with `, `.
pub fn caller_harness_names() -> String {
    let mut ids: Vec<&str> = builtins().caller_shapes().map(|(id, _)| id).collect();
    ids.sort_unstable();
    ids.join(", ")
}

/// A paneless harness process (a managed daemon): no pane-bound caller can sit under it.
pub fn is_managed_daemon(record: &ProcessRecord) -> bool {
    builtins()
        .caller_shapes()
        .any(|(_, shape)| shape.paneless.is_some_and(|p| (p.matches)(record)))
}

/// The declared caller shape of the registered adapter `id`.
fn shape_of(id: &str) -> Option<CallerEnvShape> {
    builtins()
        .caller_shapes()
        .find_map(|(shape_id, shape)| (shape_id == id).then_some(shape))
}

/// The pid a harness exported in its `pid_var`, when the environment names one.
fn exported_pid(shape: &CallerEnvShape, env: &dyn Fn(&str) -> Option<String>) -> Option<u32> {
    env(shape.pid_var?)?.trim().parse().ok()
}

/// Which registered harness a process looks like: the first declared shape whose exported pid is
/// this process (looked up in `env`), whose process names include its argv[0] basename, or whose
/// process match accepts it.
pub fn classify(record: &ProcessRecord, env: Option<EnvLookup<'_>>) -> Option<&'static str> {
    let argv0 = record.argv.first().map_or("", |a| basename(a));
    builtins().caller_shapes().find_map(|(id, shape)| {
        let exported = env.and_then(|env| exported_pid(&shape, env));
        (Some(record.pid) == exported
            || shape.process_names.contains(&argv0)
            || shape.process_match.is_some_and(|matches| matches(record)))
        .then_some(id)
    })
}

fn env_var(inputs: &EnvInputs<'_>, name: &str) -> Option<String> {
    (inputs.env)(name).filter(|v| !v.is_empty())
}

/// Nearest ancestor. `want == None`: the innermost harness of any kind. `Some(h)`: the nearest of `h`.
fn walk(
    inputs: &EnvInputs<'_>,
    want: Option<&str>,
) -> Result<(&'static str, ProcessRecord), EnvRefusal> {
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
    // Test builds: a stand-in agent. `HT_TEST_HARNESS=<registered harness id>` makes the spawning test
    // process (this one's parent) the harness, so a test that drives the binary with
    // `--cooperative-*` flags need not build a real process tree (see `test_support::isolation::
    // as_agent`). Never compiled into a release binary.
    #[cfg(any(test, feature = "test-support"))]
    if let Some(kind) = env_var(inputs, "HT_TEST_HARNESS") {
        // A hook names its own harness, which may be a synthetic registered adapter.
        let registered = builtins()
            .caller_shapes()
            .find_map(|(id, _)| (id == kind).then_some(id));
        let kind: &'static str = match (want, registered) {
            (_, Some(id)) => id,
            (Some(_), None) => "stand-in",
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
        if let Some(kind) = classify(&record, Some(inputs.env))
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
    // A harness that exports its own pid: prefer that record when it still exists.
    let record = shape_of(kind)
        .and_then(|shape| exported_pid(&shape, inputs.env))
        .and_then(|pid| inputs.processes.process(pid))
        .unwrap_or_else(|| ancestor.clone());
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
    // The harness's declared shape: its session variable, and its local evidence of the
    // session's role (a child names its parent; undecidable fails toward child).
    let shape = shape_of(kind).ok_or(EnvRefusal::NoHarness)?;
    let agent = builtins().agent(kind).map_err(|_| EnvRefusal::NoHarness)?;
    let harness = Harness::Agent(agent);
    let native = session(inputs, kind, shape.session_var)?;
    let unattributed = |detail: String| EnvRefusal::UnattributedSession {
        harness: kind,
        detail,
    };
    let hook_agent = builtins()
        .binding_policy(kind)
        .hook_agent_id
        .then(|| inputs.agent_id.clone())
        .flatten();
    let from_hook = hook_agent.is_some();
    let (native_session, agent_id) = if let Some(agent_id) = hook_agent {
        (native, Some(agent_id))
    } else {
        match shape.role.map(|role| role(native.as_str(), inputs.env)) {
            None | Some(CallerSession::TopLevel) => (native, None),
            Some(CallerSession::Subagent { parent }) => {
                let parent = NativeSessionId::parse(parent)
                    .map_err(|_| unattributed("invalid parent session id".into()))?;
                let child = AgentId::parse(native.as_str().to_owned())
                    .map_err(|_| unattributed("child session id is not a valid agent id".into()))?;
                (parent, Some(child))
            }
            // Fail closed: a distinct but unparseable marker is refused, never downgraded to
            // the top-level session.
            Some(CallerSession::Marked { agent }) => {
                let agent = AgentId::parse(agent)
                    .map_err(|_| unattributed("subagent id is not a valid agent id".into()))?;
                (native, Some(agent))
            }
            Some(CallerSession::MalformedMarker { var }) => {
                return Err(EnvRefusal::MalformedSubagentMarker { harness: kind, var });
            }
            Some(CallerSession::Unattributable(detail)) => return Err(unattributed(detail)),
        }
    };
    let fingerprint = fingerprint_for(inputs, kind, &ancestor)?;
    let provenance = if from_hook {
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

/// The nearest ancestor of the given harness (a registered harness id): hooks know their
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
