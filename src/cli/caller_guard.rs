//! The two-family caller guard: a caller whose environment names both a Herdr pane and a
//! herdr-core terminal is verified by the daemon (`LocateCaller`) before anything binds.
//!
//! Owns: sending `LocateCaller` for an `Ambiguous` caller (never for a `One` caller:
//! verifying those would be adversarial, TRUST-POLICY A1), turning the reply into the
//! verified family or a refusal, and the exact text of the refusal line M4 (CLI stderr and
//! hook stderr). Does not own: reading the environment (`caller_location`) or the walk
//! itself (`service::locate`).
//! Where new behaviour goes: a verified family is a `Guard` variant; what a family may do
//! once verified is that family's `CallerSite` answers.

use super::caller_location::{CallerEnv, CallerLocator, EnvMode, Located};
use crate::{
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
    /// The Herdr pane is the nearest ancestor root: it wins.
    Pane(String),
    /// The herdr-core terminal is the nearest ancestor root: it wins. It names no pane, so
    /// nothing binds to the pane the environment also named.
    Terminal,
    /// Nothing verified: no seat binds.
    Refused(Refusal),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The terminal as `<session>:<tN>`.
    pub terminal: String,
    /// The Herdr pane the environment also named.
    pub pane: String,
}

impl Refusal {
    /// M4: 106 bytes + the pane and the terminal address. CLI stderr and hook stderr.
    pub fn m4(&self) -> String {
        format!(
            "herdr-threads: env names pane {} and terminal {}; neither is an ancestor of this process; unset the stale one",
            self.pane, self.terminal
        )
    }
}

/// Asks the daemon which of `candidates` runs this process. Any failure to ask (older
/// daemon, unreachable daemon) is an unverified caller: the guard fails closed.
pub fn verify(
    client: &dyn LocalClient,
    candidates: &[CallerLocator],
    budget: &CallBudget,
) -> Guard {
    let refused = || refusal(candidates);
    if !client.supports_capability(LOCATE_CALLER_V1, budget) {
        return refused();
    }
    let request = LocateCaller {
        candidates: candidates
            .iter()
            .filter_map(|c| c.site().locate_candidate())
            .collect(),
        subject: None,
    };
    match client.call(Command::LocateCaller(request), budget) {
        Ok(CommandResult::Located(LocateReply::Verified { index, .. })) => {
            match candidates.get(index) {
                Some(CallerLocator::Pane { id }) => Guard::Pane(id.clone()),
                Some(CallerLocator::Terminal { .. }) => Guard::Terminal,
                Some(CallerLocator::Named(_)) | None => refused(),
            }
        }
        _ => refused(),
    }
}

/// The pane a CLI call acts from. One family (or none) is used as is and the daemon is
/// never asked; an `Ambiguous` caller goes through `verify_with`. When nothing verifies
/// `report` receives M4 and no pane is returned (nothing binds); a verified terminal names no
/// pane either.
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
        Guard::Terminal => None,
        Guard::Refused(refusal) => {
            report(&refusal.m4());
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
pub fn unasked(candidates: &[CallerLocator]) -> Guard {
    refusal(candidates)
}

fn refusal(candidates: &[CallerLocator]) -> Guard {
    let find = |pick: fn(&CallerLocator) -> Option<String>| {
        candidates.iter().find_map(pick).unwrap_or_default()
    };
    Guard::Refused(Refusal {
        pane: find(|c| c.site().relative_pane().map(str::to_owned)),
        terminal: find(|c| c.site().terminal_address()),
    })
}
