//! Caller location: the CLI's only reader of caller-location environment.
//!
//! Pattern: a value type with closed enums (`CallerEnv` -> `Located` ->
//! `CallerSite`), built once per CLI or hook invocation and immutable.
//!
//! Owns: reading and empty-filtering the caller-location variables (the names
//! come from each host module's env consts, never from a literal here), the
//! precedence between families (one family, then `Ambiguous`, then refusal),
//! and the per-site answers (`moved_target`, `relative_pane`,
//! `person_origin_allowed`, `enrolls_on_first_use`, `attempts_continuity`,
//! `host_scoped_lookup`, `cache_key`, `env_assignments`) as one closed match.
//!
//! Hides: which env family located the caller, from `mod.rs`, `hook.rs`,
//! the handoff recovery hints and instance discovery.
//!
//! Does not: verify anything (that is the daemon's call against the canonical
//! view, A2), or rank one family above another. `Ambiguous` is passed on
//! as is; a caller that cannot yet verify uses the pane candidate unverified,
//! exactly as before a second family existed.
//!
//! Where new behaviour goes: a new site kind is a `CallerLocator` variant plus
//! an arm in each `CallerSite` method and in `CallerEnv::locate`. No other
//! `src/cli` file reads these variables; `tests/cli_env_read_scan.rs` enforces
//! it. F lands `Pane` and a recognition-only `Terminal` (so a second family
//! is visible to the phase guard); `Named` and the verified `Terminal` come
//! with their phases.

use std::collections::HashMap;
use std::ffi::OsString;

use crate::host::{HC_TERMINAL_ID_ENV, HC_TERMINAL_SOCKET_ENV, native};

/// Which entry point is asking. A hook only trusts the pane family when Herdr
/// itself marked the environment (`HERDR_ENV=1`); the CLI accepts a pane id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvMode {
    Cli,
    Hook,
}

/// One family's evidence about where the caller lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerLocator {
    Pane {
        id: String,
    },
    /// Recognition only in F: it makes a second family visible, nothing
    /// consumes its values yet.
    Terminal {
        socket: String,
        id: String,
    },
}

impl CallerLocator {
    pub fn pane(id: impl Into<String>) -> Self {
        Self::Pane { id: id.into() }
    }

    pub fn site(&self) -> CallerSite<'_> {
        CallerSite(self)
    }
}

/// The outcome of the precedence: one family, or several for the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Located {
    One(CallerLocator),
    Ambiguous(Vec<CallerLocator>),
}

impl Located {
    /// The pane id a pane-relative lookup uses. Until the daemon verifies
    /// `Ambiguous` candidates, the pane candidate is used unverified, as it
    /// was before a second family existed.
    pub fn relative_pane(&self) -> Option<&str> {
        match self {
            Self::One(locator) => locator.site().relative_pane(),
            Self::Ambiguous(all) => all.iter().find_map(|l| l.site().relative_pane()),
        }
    }
}

/// Why the environment cannot locate a caller. Kept distinct so each can carry
/// its own text when its phase lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocatorError {
    /// A terminal id with no socket and no other family.
    TerminalWithoutSocket,
}

/// The caller-location variables of one process, empty values already dropped.
#[derive(Debug, Clone, Default)]
pub struct CallerEnv {
    vars: HashMap<&'static str, OsString>,
}

impl CallerEnv {
    pub fn from_process() -> Self {
        Self::from_lookup(|name| std::env::var_os(name))
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<OsString>) -> Self {
        let names = native::ENV_VARS
            .iter()
            .chain([&HC_TERMINAL_ID_ENV, &HC_TERMINAL_SOCKET_ENV]);
        let vars = names
            .filter_map(|&name| {
                lookup(name)
                    .filter(|value| !value.is_empty())
                    .map(|value| (name, value))
            })
            .collect();
        Self { vars }
    }

    fn text(&self, name: &str) -> Option<String> {
        self.vars.get(name)?.to_str().map(str::to_owned)
    }

    /// Herdr marked this environment as one of its panes (`HERDR_ENV=1`).
    pub fn herdr_env(&self) -> bool {
        self.vars
            .get(native::ENV_HERDR)
            .is_some_and(|value| value == "1")
    }

    pub fn locate(&self, mode: EnvMode) -> Result<Option<Located>, LocatorError> {
        let pane = self
            .text(native::ENV_PANE_ID)
            .filter(|_| mode == EnvMode::Cli || self.herdr_env())
            .map(|id| CallerLocator::Pane { id });
        let terminal = match (
            self.text(HC_TERMINAL_ID_ENV),
            self.text(HC_TERMINAL_SOCKET_ENV),
        ) {
            (Some(id), Some(socket)) => Some(CallerLocator::Terminal { socket, id }),
            (Some(_), None) if pane.is_none() => return Err(LocatorError::TerminalWithoutSocket),
            _ => None,
        };
        Ok(match (pane, terminal) {
            (Some(pane), Some(terminal)) => Some(Located::Ambiguous(vec![pane, terminal])),
            (Some(one), None) | (None, Some(one)) => Some(Located::One(one)),
            (None, None) => None,
        })
    }

    /// The pane the caller runs in, or `None` when the environment locates no
    /// pane (including a refusal, which F reports as no caller).
    pub fn pane(&self, mode: EnvMode) -> Option<String> {
        let located = self.locate(mode).ok().flatten()?;
        located.relative_pane().map(str::to_owned)
    }
}

/// The closed match of answers for one located site.
#[derive(Debug, Clone, Copy)]
pub struct CallerSite<'a>(&'a CallerLocator);

impl<'a> CallerSite<'a> {
    /// The host target a handoff or watch follows when the caller moves.
    pub fn moved_target(self) -> Option<&'a str> {
        match self.0 {
            CallerLocator::Pane { id } | CallerLocator::Terminal { id, .. } => Some(id),
        }
    }

    /// The pane id a pane-relative seat lookup takes.
    pub fn relative_pane(self) -> Option<&'a str> {
        match self.0 {
            CallerLocator::Pane { id } => Some(id),
            CallerLocator::Terminal { .. } => None,
        }
    }

    /// A person can enroll from this site (`human me init`).
    pub fn person_origin_allowed(self) -> bool {
        matches!(self.0, CallerLocator::Pane { .. })
    }

    /// A target with no seat yet is registered by its first hook or call.
    pub fn enrolls_on_first_use(self) -> bool {
        matches!(self.0, CallerLocator::Pane { .. })
    }

    /// The daemon may re-attach a seat after the host restarted.
    pub fn attempts_continuity(self) -> bool {
        matches!(self.0, CallerLocator::Pane { .. })
    }

    /// Seat lookup is scoped to this site's host.
    pub fn host_scoped_lookup(self) -> bool {
        matches!(self.0, CallerLocator::Pane { .. })
    }

    /// A stable key for a per-site cache.
    pub fn cache_key(self) -> String {
        match self.0 {
            CallerLocator::Pane { id } => format!("pane:{id}"),
            CallerLocator::Terminal { socket, id } => format!("terminal:{socket}:{id}"),
        }
    }

    /// `NAME=value` words that re-establish this site for a recovery command.
    pub fn env_assignments(self) -> Vec<String> {
        match self.0 {
            CallerLocator::Pane { id } => vec![format!("{}={id}", native::ENV_PANE_ID)],
            CallerLocator::Terminal { socket, id } => vec![
                format!("{HC_TERMINAL_ID_ENV}={id}"),
                format!("{HC_TERMINAL_SOCKET_ENV}={socket}"),
            ],
        }
    }
}

/// The `NAME=value` words a handoff recovery command carries to act from the
/// frozen claim's pane.
pub fn recovery_env(pane: &str) -> Vec<String> {
    CallerLocator::pane(pane).site().env_assignments()
}
