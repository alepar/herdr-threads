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
//! `host_scoped_lookup`, `claim_host_locator`, `cache_key`, `env_assignments`) as one closed match.
//!
//! Hides: which env family located the caller, from `mod.rs`, `hook.rs`,
//! the handoff recovery hints and instance discovery.
//!
//! Does not: verify anything (that is the daemon's call against the canonical
//! view, A2), or rank one family above another. `Ambiguous` is passed on
//! as is; until it is verified (`caller_guard`), the pane candidate serves a
//! pane-relative lookup unverified, exactly as before a second family existed.
//!
//! Where new behaviour goes: a new site kind is a `CallerLocator` variant plus
//! an arm in each `CallerSite` method and in `CallerEnv::locate`. No other
//! `src/cli` file reads these variables; `tests/cli_env_read_scan.rs` enforces
//! it. `Pane`, `Terminal` (a herdr-core terminal) and `Named` (precedence rule 1: a valid
//! seat name replaces only the target, so it is never part of an `Ambiguous` pair).

use std::collections::HashMap;
use std::ffi::OsString;

use crate::host::{HC_TERMINAL_ID_ENV, HC_TERMINAL_SOCKET_ENV, named, native};
use crate::protocol::ids::{HostLocator, HostTargetId, NAMED_HOST_LOCATOR, SeatName};

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
    /// A herdr-core terminal (`HC_TERMINAL_ID` with `HC_TERMINAL_SOCKET`); beside a pane it is
    /// one of two families the daemon's `LocateCaller` tells apart.
    Terminal {
        socket: String,
        id: String,
    },
    /// A named seat (`HERDR_THREADS_SEAT_NAME`): the name is the target on the built-in named
    /// host. It needs no Herdr environment.
    Named(SeatName),
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
    /// `HERDR_THREADS_SEAT_NAME` is set and is not a valid seat name.
    InvalidName,
}

impl LocatorError {
    /// The one-line CLI refusal for this error.
    pub fn refusal(&self) -> Option<&'static str> {
        match self {
            Self::InvalidName => Some(M16),
            Self::TerminalWithoutSocket => Some(M17),
        }
    }
}

/// M14 (63 bytes + the flag): an explicit cooperative flag beside a seat name.
pub fn m14(flag: &str) -> String {
    format!(
        "herdr-threads: {} cannot be combined with {flag}",
        named::ENV_SEAT_NAME
    )
}

/// M15 (82 bytes): `me init` under a seat name.
pub const M15: &str =
    "herdr-threads: named person seats are not supported; unset HERDR_THREADS_SEAT_NAME";

/// M16 (92 bytes): `HERDR_THREADS_SEAT_NAME` is set and is not a valid seat name.
pub const M16: &str =
    "herdr-threads: HERDR_THREADS_SEAT_NAME is not a valid seat name (1-64 bytes of A-Za-z0-9._-)";

/// M17 (91 bytes): `HC_TERMINAL_ID` is the only evidence and names no core to ask.
pub const M17: &str =
    "herdr-threads: HC_TERMINAL_ID is set without HC_TERMINAL_SOCKET; cannot locate the terminal";

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
            .chain(named::ENV_VARS)
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

    /// The environment marks the binding read-only (`HERDR_THREADS_SEAT_READONLY=1`, D3).
    pub fn readonly(&self) -> bool {
        self.vars
            .get(named::ENV_SEAT_READONLY)
            .is_some_and(|value| value == "1")
    }

    /// The valid seat name the environment carries; `None` when absent or invalid
    /// ([`CallerEnv::locate`] tells those apart).
    pub fn named(&self) -> Option<SeatName> {
        SeatName::parse(self.text(named::ENV_SEAT_NAME)?).ok()
    }

    pub fn locate(&self, mode: EnvMode) -> Result<Option<Located>, LocatorError> {
        // Precedence rule 1: a valid name replaces only the target, in either mode and with no
        // Herdr environment; an invalid one is a hard error.
        if self.vars.contains_key(named::ENV_SEAT_NAME) {
            return match self.named() {
                Some(name) => Ok(Some(Located::One(CallerLocator::Named(name)))),
                None => Err(LocatorError::InvalidName),
            };
        }
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

    /// The caller is a lone non-Pane site (a named seat or a herdr-core terminal): it has no
    /// Herdr pane to ask Herdr about, so instance discovery treats it alike (SA1).
    pub fn non_pane_site(&self) -> bool {
        matches!(
            self.locate(EnvMode::Cli),
            Ok(Some(Located::One(
                CallerLocator::Named(_) | CallerLocator::Terminal { .. }
            )))
        )
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
            CallerLocator::Named(name) => Some(name.as_str()),
        }
    }

    /// The pane id a pane-relative seat lookup takes.
    pub fn relative_pane(self) -> Option<&'a str> {
        match self.0 {
            CallerLocator::Pane { id } => Some(id),
            CallerLocator::Terminal { .. } | CallerLocator::Named(_) => None,
        }
    }

    /// A person can enroll from this site (`human me init`).
    pub fn person_origin_allowed(self) -> bool {
        matches!(self.0, CallerLocator::Pane { .. })
    }

    /// A target with no seat yet is registered by its first hook or call.
    pub fn enrolls_on_first_use(self) -> bool {
        matches!(self.0, CallerLocator::Pane { .. } | CallerLocator::Named(_))
    }

    /// The daemon may re-attach a seat after the host restarted.
    pub fn attempts_continuity(self) -> bool {
        matches!(self.0, CallerLocator::Pane { .. })
    }

    /// Seat lookup is scoped to this site's host.
    pub fn host_scoped_lookup(self) -> bool {
        matches!(self.0, CallerLocator::Pane { .. } | CallerLocator::Named(_))
    }

    /// The host and target a request from this site carries, only for a site that sends them:
    /// a named seat is its name at the reserved named-host locator. A Pane site sends neither
    /// (its request bytes are unchanged).
    pub fn sent_scope(self) -> Option<(HostLocator, HostTargetId)> {
        match self.0 {
            CallerLocator::Named(name) => Some((
                HostLocator::parse(NAMED_HOST_LOCATOR).expect("the reserved locator parses"),
                HostTargetId::new(name.as_str()),
            )),
            CallerLocator::Pane { .. } | CallerLocator::Terminal { .. } => None,
        }
    }

    /// This site as the daemon's `LocateCaller` candidate. A named site is never a candidate:
    /// it is never one of several families.
    pub fn locate_candidate(self) -> Option<crate::protocol::locate::LocateCandidate> {
        use crate::protocol::locate::LocateCandidate;
        match self.0 {
            CallerLocator::Pane { id } => Some(LocateCandidate::Pane { id: id.clone() }),
            CallerLocator::Terminal { socket, id } => Some(LocateCandidate::Terminal {
                socket: socket.clone(),
                id: id.clone(),
            }),
            CallerLocator::Named(_) => None,
        }
    }

    /// The terminal as the refusal line prints it, `<session>:<tN>`; the session is the core
    /// socket's file stem (herdr-core names its socket `<session>.sock`), omitted when the
    /// socket has none. `None` for other sites.
    pub fn terminal_address(self) -> Option<String> {
        match self.0 {
            CallerLocator::Terminal { socket, id } => {
                let session = std::path::Path::new(socket)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .filter(|stem| !stem.is_empty());
                Some(session.map_or_else(|| id.clone(), |session| format!("{session}:{id}")))
            }
            CallerLocator::Pane { .. } | CallerLocator::Named(_) => None,
        }
    }

    /// The claim host a hook from this site uses in place of the installed argv's
    /// `--host-endpoint` (a Herdr server): a named seat's reserved host, a terminal's core
    /// socket. A Pane site has none (its own `HERDR_SOCKET_PATH` decides).
    pub fn claim_host_locator(self) -> Option<&'a str> {
        match self.0 {
            CallerLocator::Named(_) => Some(NAMED_HOST_LOCATOR),
            CallerLocator::Terminal { socket, .. } => Some(socket),
            CallerLocator::Pane { .. } => None,
        }
    }

    /// A stable key for a per-site cache.
    pub fn cache_key(self) -> String {
        match self.0 {
            CallerLocator::Pane { id } => format!("pane:{id}"),
            CallerLocator::Terminal { socket, id } => format!("terminal:{socket}:{id}"),
            CallerLocator::Named(name) => format!("named:{}", name.as_str()),
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
            CallerLocator::Named(name) => {
                vec![format!("{}={}", named::ENV_SEAT_NAME, name.as_str())]
            }
        }
    }
}

/// The `NAME=value` words a handoff recovery command carries to act from the
/// frozen claim's pane.
pub fn recovery_env(pane: &str) -> Vec<String> {
    CallerLocator::pane(pane).site().env_assignments()
}
