//! The two-family caller guard (F): a caller whose environment names both a Herdr pane
//! and a herdr-core terminal is verified by the daemon (`LocateCaller`) before anything
//! binds to the pane.
//!
//! Owns: sending `LocateCaller` for an `Ambiguous` caller (never for a `One` caller:
//! verifying those would be adversarial, TRUST-POLICY A1), turning the reply into a
//! pane or a refusal, and the exact text of the refusal lines M1 (CLI and hook stderr)
//! and M3 (hook context). Does not own: reading the environment (`caller_location`), the
//! walk itself (`service::locate`), or when a hook injects M3 (`hook.rs`).
//! H1 removes this file with M1 and M3: a two-family caller then goes to the hc candidate.

use super::caller_location::{CallerEnv, CallerLocator, EnvMode, Located};
use crate::{
    host::named,
    ports::LocalClient,
    protocol::{
        capabilities::LOCATE_CALLER_V1,
        commands::Command,
        locate::{LocateCaller, LocateReply},
        results::CommandResult,
        time::CallBudget,
    },
};

/// What the daemon made of an `Ambiguous` caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guard {
    /// The Herdr pane is an ancestor of this process: it wins.
    Pane(String),
    /// Nothing verified: no seat binds. `first` is the daemon's "first time for this
    /// guard key" answer.
    Refused(Refusal),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The terminal id (`tN`).
    pub terminal: String,
    /// The Herdr pane the environment also named.
    pub pane: String,
    pub first: bool,
}

/// M2: the 29-byte suffix on M1 and M3 that names the way out (a named seat). H1 removes it
/// with them.
pub fn m2() -> String {
    format!("; set {}", named::ENV_SEAT_NAME)
}

impl Refusal {
    /// M1 + M2: 62 + 29 bytes + the terminal id. CLI stderr and hook stderr.
    pub fn m1(&self) -> String {
        format!(
            "herdr-threads: runs in herdr-core terminal {}; not supported yet{}",
            self.terminal,
            m2()
        )
    }

    /// M3 + M2: 88 + 29 bytes + terminal id + pane id. Hook context (check-ins only).
    pub fn m3(&self) -> String {
        format!(
            "[herdr-threads] guard: runs in herdr-core terminal {}, not Herdr pane {}; no seat bound here{}",
            self.terminal,
            self.pane,
            m2()
        )
    }
}

/// Asks the daemon which of `candidates` runs this process. Any failure to ask (older
/// daemon, unreachable daemon) is an unverified caller: the guard fails closed.
pub fn verify(
    client: &dyn LocalClient,
    candidates: &[CallerLocator],
    guard_key: Option<String>,
    budget: &CallBudget,
) -> Guard {
    let pane = candidates
        .iter()
        .find_map(|c| c.site().relative_pane())
        .unwrap_or_default()
        .to_owned();
    let terminal = candidates
        .iter()
        .find_map(|c| c.site().terminal_id())
        .unwrap_or_default()
        .to_owned();
    let refused = |first| {
        Guard::Refused(Refusal {
            terminal: terminal.clone(),
            pane: pane.clone(),
            first,
        })
    };
    if !client.supports_capability(LOCATE_CALLER_V1, budget) {
        return refused(false);
    }
    let request = LocateCaller {
        candidates: candidates
            .iter()
            .filter_map(|c| c.site().locate_candidate())
            .collect(),
        subject: None,
        guard_key,
    };
    match client.call(Command::LocateCaller(request), budget) {
        Ok(CommandResult::Located(LocateReply::Verified { index, .. })) => candidates
            .get(index)
            .and_then(|c| c.site().relative_pane())
            .map_or_else(|| refused(false), |pane| Guard::Pane(pane.to_owned())),
        Ok(CommandResult::Located(LocateReply::Unverified { first_guard, .. })) => {
            refused(first_guard)
        }
        _ => refused(false),
    }
}

/// The pane a CLI call acts from. One family (or none) is used as is and the daemon is
/// never asked; an `Ambiguous` caller goes through `verify_with`, and when nothing verifies
/// `report` receives M1 and no pane is returned (nothing binds).
pub fn cli_pane(
    env: &CallerEnv,
    verify_with: impl FnOnce(&[CallerLocator]) -> Guard,
    report: impl FnOnce(&str),
) -> Option<String> {
    let Ok(Some(Located::Ambiguous(candidates))) = env.locate(EnvMode::Cli) else {
        return env.pane(EnvMode::Cli);
    };
    match verify_with(&candidates) {
        Guard::Pane(pane) => Some(pane),
        Guard::Refused(refusal) => {
            report(&refusal.m1());
            None
        }
    }
}

/// The candidates a hook must have verified: set only for an `Ambiguous` caller.
pub fn hook_candidates(env: &CallerEnv) -> Option<Vec<CallerLocator>> {
    match env.locate(EnvMode::Hook) {
        Ok(Some(Located::Ambiguous(all))) => Some(all),
        _ => None,
    }
}

/// A refusal for a caller the guard could not even ask about (no daemon to connect to).
pub fn unasked(candidates: &[CallerLocator], pane: &str) -> Guard {
    Guard::Refused(Refusal {
        terminal: candidates
            .iter()
            .find_map(|c| c.site().terminal_id())
            .unwrap_or_default()
            .to_owned(),
        pane: pane.to_owned(),
        first: false,
    })
}
