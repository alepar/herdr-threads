//! `internal launch-gate --bootstrap <compound>:<attempt> -- <argv>`: the shim a herdr-core
//! `terminal.run` starts instead of the harness (H3b, TRUST-POLICY H3 `managed_launch`).
//!
//! Responsibility
//! - Owns: blanking the leaked environment, waiting (bounded) for the daemon to authorize the
//!   terminal's `(core, tN)` witness against the bootstrap, then `exec`ing the harness argv.
//! - Hides: which names are scrubbed (`host::gate_shim_scrub_list`, `HERDR_*`, each adapter's
//!   `CallerEnvShape` variables and `child_scrub_vars`) and how a config root becomes env
//!   (the adapter's `explicit_directory_env`).
//! - Must not: decide authorization (the daemon does, against the canonical view, A2), keep
//!   `HC_TERMINAL_ID`/`HC_TERMINAL_SOCKET`/`HC_SCOPE` out of the exec'd env (the worker locates
//!   its terminal seat through them), or fork: `exec` keeps the pid (L2), which keeps the
//!   two-phase launch invariant's root-pid fingerprint.

use std::{
    collections::BTreeSet, ffi::OsString, os::unix::process::CommandExt,
    process::Command as ProcessCommand, sync::Arc,
};

use super::{
    RunError,
    caller_location::{CallerEnv, CallerLocator, EnvMode, Located},
    invalid_request,
};
use crate::{
    ports::LocalClient,
    protocol::{
        commands::Command,
        handoff::{AuthorizeLaunchGate, BootstrapAttempt, LaunchGateRefusal, LaunchGateReply},
        ids::OperationId,
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};

/// How long the shim asks the daemon to wait for the bootstrap Send to be staged (ms).
pub const GATE_TIMEOUT_MS: u32 = 10_000;
/// Transport slack beyond the daemon's own wait before the call is abandoned (ms).
const CALL_SLACK_MS: u64 = 2_000;

/// What the shim authorizes: its bootstrap and the terminal it runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub compound: OperationId,
    pub attempt: BootstrapAttempt,
    /// The core socket (`HC_TERMINAL_SOCKET`).
    pub socket: String,
    /// The terminal id (`HC_TERMINAL_ID`, `tN`).
    pub terminal: String,
}

/// `<compound>:<attempt>`; the attempt is the text after the last colon.
pub fn parse_bootstrap(value: &str) -> Result<(OperationId, BootstrapAttempt), RunError> {
    let bad = || invalid_request("internal launch-gate --bootstrap needs <compound>:<attempt>");
    let (compound, attempt) = value.rsplit_once(':').ok_or_else(bad)?;
    let compound = OperationId::parse(compound).map_err(|_| bad())?;
    let attempt = attempt
        .parse::<u32>()
        .ok()
        .and_then(|n| BootstrapAttempt::new(n).ok())
        .ok_or_else(bad)?;
    Ok((compound, attempt))
}

/// The gate for `bootstrap`, from the terminal the shim runs in (`HC_TERMINAL_*`, read through
/// [`CallerEnv`]; a stray Herdr pane beside it does not hide the terminal).
pub fn gate_from_env(bootstrap: &str, env: &CallerEnv) -> Result<Gate, RunError> {
    let (compound, attempt) = parse_bootstrap(bootstrap)?;
    let located = env
        .locate(EnvMode::Cli)
        .map_err(|error| invalid_request(error.refusal().unwrap_or("caller not locatable")))?;
    let candidates = match located {
        Some(Located::One(one)) => vec![one],
        Some(Located::Ambiguous(all)) => all,
        None => Vec::new(),
    };
    candidates
        .into_iter()
        .find_map(|locator| match locator {
            CallerLocator::Terminal { socket, id } => Some(Gate {
                compound: compound.clone(),
                attempt,
                socket,
                terminal: id,
            }),
            _ => None,
        })
        .ok_or_else(|| invalid_request("internal launch-gate must run in a herdr-core terminal"))
}

fn refusal_text(reason: LaunchGateRefusal) -> &'static str {
    match reason {
        LaunchGateRefusal::UnknownBootstrap => "unknown bootstrap",
        LaunchGateRefusal::WitnessMismatch => "terminal is not the bootstrap's recipient",
        LaunchGateRefusal::StaleAttempt => "stale attempt",
        LaunchGateRefusal::TimedOut => "timed out waiting for the bootstrap",
    }
}

/// Ask the daemon to authorize `(core_id, gate.terminal)` and wait at most `timeout_ms` (plus
/// transport slack). `Ok` only on `Authorized`; every refusal, transport error or deadline is an
/// error, so the caller never `exec`s a harness the daemon did not release.
pub fn authorize<C: LocalClient + ?Sized>(
    client: &C,
    gate: &Gate,
    core_id: &str,
    timeout_ms: u32,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    let request = AuthorizeLaunchGate {
        compound: gate.compound.clone(),
        attempt: gate.attempt,
        core_id: core_id.to_owned(),
        terminal: gate.terminal.clone(),
        timeout_ms,
        socket: Some(gate.socket.clone()),
    };
    request.validate().map_err(invalid_request)?;
    let budget = CallBudget {
        deadline: MonoInstant(
            clock
                .monotonic_now()
                .0
                .saturating_add(u64::from(timeout_ms) + CALL_SLACK_MS),
        ),
        cancellation: Cancellation::default(),
    };
    match client.call(Command::AuthorizeLaunchGate(Box::new(request)), &budget)? {
        CommandResult::LaunchGate(LaunchGateReply::Authorized) => Ok(()),
        CommandResult::LaunchGate(LaunchGateReply::Refused { reason }) => {
            let code = if reason == LaunchGateRefusal::TimedOut {
                ErrorCode::DeadlineExceeded
            } else {
                ErrorCode::Unauthorized
            };
            Err(ApiError::new(
                code,
                format!("launch gate refused: {}", refusal_text(reason)),
            )
            .into())
        }
        _ => Err(invalid_request("unexpected launch gate result")),
    }
}

/// Every name the shim removes from the exec'd env, given the names `ambient` holds: the gate
/// list (K5 minus the hc terminal variables), every ambient `HERDR_*`, each adapter's session
/// and pid variables and its `child_scrub_vars`.
pub fn scrub_names(ambient: impl IntoIterator<Item = OsString>) -> BTreeSet<OsString> {
    let mut names: BTreeSet<OsString> = crate::host::gate_shim_scrub_list()
        .into_iter()
        .map(OsString::from)
        .collect();
    names.extend(
        ambient
            .into_iter()
            .filter(|name| name.to_str().is_some_and(|n| n.starts_with("HERDR_"))),
    );
    for registration in crate::harness::registry::builtins().registrations() {
        if let Some(shape) = registration.caller_env() {
            names.insert(shape.session_var.into());
            names.extend(shape.pid_var.map(OsString::from));
        }
        names.extend(registration.child_scrub_vars().iter().map(OsString::from));
    }
    names
}

/// The harness command: `argv` with the env blanked and each `HARNESS=DIR` config root applied
/// through that adapter's `explicit_directory_env` (set after the removal, so it wins over a
/// leaked value).
pub fn prepare(
    argv: &[String],
    config_roots: &[String],
    ambient: impl IntoIterator<Item = OsString>,
) -> Result<ProcessCommand, RunError> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| invalid_request("internal launch-gate needs a harness argv after --"))?;
    let mut command = ProcessCommand::new(program);
    command.args(args);
    for name in scrub_names(ambient) {
        command.env_remove(name);
    }
    for entry in config_roots {
        let (harness, dir) = entry
            .split_once('=')
            .filter(|(h, d)| !h.is_empty() && std::path::Path::new(d).is_absolute())
            .ok_or_else(|| {
                invalid_request("internal launch-gate --config-root needs HARNESS=ABSOLUTE_DIR")
            })?;
        let registration = crate::harness::registry::builtins()
            .registrations()
            .iter()
            .find(|r| r.metadata().id == harness)
            .ok_or_else(|| {
                invalid_request("internal launch-gate --config-root: unknown harness")
            })?;
        let policy = registration.launch_policy().ok_or_else(|| {
            invalid_request("internal launch-gate --config-root: no launch policy")
        })?;
        for (name, value) in policy.explicit_directory_env(std::path::Path::new(dir))? {
            command.env(name, value);
        }
    }
    Ok(command)
}

/// Authorize, then `exec` the harness. Returns only on failure (a successful `exec` replaces
/// this process, keeping its pid).
#[allow(clippy::too_many_arguments)]
pub fn run_with<C: LocalClient + ?Sized>(
    client: &C,
    clock: &dyn Clock,
    env: &CallerEnv,
    bootstrap: &str,
    config_roots: &[String],
    argv: &[String],
    ambient: Vec<OsString>,
    timeout_ms: u32,
    core_id: impl FnOnce(&str) -> Result<String, ApiError>,
) -> RunError {
    let outcome = (|| {
        let gate = gate_from_env(bootstrap, env)?;
        let mut command = prepare(argv, config_roots, ambient)?;
        let core = core_id(&gate.socket)?;
        authorize(client, &gate, &core, timeout_ms, clock)?;
        Ok::<_, RunError>(command.exec())
    })();
    match outcome {
        Ok(error) => RunError::Io(error),
        Err(error) => error,
    }
}

/// The shim's view of its own env: only the herdr-core terminal variables count. A leaked
/// `HERDR_*` (a seat name, a pane) must not hide the terminal behind precedence rule 1 or 3.
pub fn terminal_env(lookup: impl Fn(&str) -> Option<OsString>) -> CallerEnv {
    CallerEnv::from_lookup(|name| {
        if name.starts_with("HERDR_") {
            None
        } else {
            lookup(name)
        }
    })
}

/// The production entry: the core's id comes from its own `hello` on the terminal's socket.
pub fn run<C: LocalClient + ?Sized>(
    client: &C,
    clock: &Arc<dyn Clock>,
    bootstrap: &str,
    config_roots: &[String],
    argv: &[String],
) -> Result<(), RunError> {
    Err(run_with(
        client,
        clock.as_ref(),
        &terminal_env(|name| std::env::var_os(name)),
        bootstrap,
        config_roots,
        argv,
        std::env::vars_os().map(|(name, _)| name).collect(),
        GATE_TIMEOUT_MS,
        |socket| {
            let host = crate::host::hc::HcHost::with_clock(
                crate::host::hc::HcClient::new(socket),
                Arc::clone(clock),
            );
            match host.core_witness()? {
                crate::ports::HostWitness::HcCore { core_id, .. } => Ok(core_id),
                _ => Err(ApiError::host_unavailable("terminal socket is not a core")),
            }
        },
    ))
}

#[cfg(all(test, feature = "test-support"))]
#[path = "../../tests/cli/launch_gate.rs"]
mod gate_tests;
