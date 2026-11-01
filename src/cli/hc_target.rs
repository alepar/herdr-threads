//! Which host a `handoff` or `launch` creates its target on: a herdr-core host, or today's Herdr tab.
//!
//! Responsibility
//! - Owns: choosing the herdr-core host of a V3 automatic handoff from the caller's site (a
//!   `Terminal` site names its own core; a plain shell with no Herdr context uses
//!   `HcDiscovery`), turning `--space`/`--new-tab`/`--new-terminal` into a `Placement`, and
//!   refusing the combinations that have no V3 record (`--thread`, `--new-thread`).
//! - Hides: that discovery reads core state files and one `hello` per core; it never runs `hc`.
//! - Must not: create anything, probe a Herdr socket, or read the process environment (the
//!   caller passes a lookup).

use std::path::PathBuf;

use super::{
    RunError,
    caller_location::CallerLocator,
    invalid_request,
    topology_handoff::{Request, TargetSelection},
};
use crate::{
    host::hc::discovery::{EnvLookup, HcDiscovery},
    ports::Placement,
    protocol::ids::HostTargetId,
};

/// What a creation asks for, read from `handoff`'s or `launch`'s flags.
pub struct Wanted<'a> {
    pub new_tab: bool,
    pub new_terminal: bool,
    pub space: Option<&'a str>,
    /// `--tab` or `--pane` beside the creation.
    pub narrowed: bool,
    /// False for `--thread` and `--new-thread`, which have no V3 record; a plain launch is true.
    pub automatic: bool,
}

/// The target a handoff creates on a herdr-core host, or `None` for the Herdr new-tab path.
///
/// `herdr_context` is true when the call carries a Herdr server (a host endpoint from the
/// environment or `--host-endpoint`): such a call is a Herdr handoff, never a discovered one.
pub fn select(
    request: &Request,
    site: Option<&CallerLocator>,
    herdr_context: bool,
    env: EnvLookup<'_>,
) -> Result<Option<TargetSelection>, RunError> {
    if request.existing || request.target.is_some() {
        return Ok(None);
    }
    select_wanted(
        &Wanted {
            new_tab: request.new_tab.is_some(),
            new_terminal: request.new_terminal,
            space: request.selector.space.as_deref(),
            narrowed: request.selector.tab.is_some() || request.selector.pane.is_some(),
            automatic: request.automatic,
        },
        site,
        herdr_context,
        env,
    )
}

/// The target a creation asks for on a herdr-core host, or `None` for the Herdr new-tab path.
pub fn select_wanted(
    wanted: &Wanted<'_>,
    site: Option<&CallerLocator>,
    herdr_context: bool,
    env: EnvLookup<'_>,
) -> Result<Option<TargetSelection>, RunError> {
    if !wanted.new_tab && !wanted.new_terminal {
        return Ok(None);
    }
    let (socket, parent) = match site {
        Some(CallerLocator::Terminal { socket, id }) => (
            Some(PathBuf::from(socket)),
            Some(HostTargetId::parse(id).map_err(invalid_request)?),
        ),
        Some(_) => {
            if wanted.new_terminal {
                return Err(invalid_request(
                    "--new-terminal needs a calling herdr-core terminal; this caller is not one",
                ));
            }
            return Ok(None);
        }
        None if herdr_context => {
            if wanted.new_terminal {
                return Err(invalid_request(
                    "--new-terminal needs a calling herdr-core terminal; this caller is not one",
                ));
            }
            return Ok(None);
        }
        None => (None, None),
    };
    if !wanted.automatic {
        return Err(invalid_request(
            "--thread and --new-thread cannot create a herdr-core terminal; \
             use the automatic handoff",
        ));
    }
    let placement = if wanted.new_terminal {
        Placement::NewTerminal {
            parent: parent.ok_or_else(|| {
                invalid_request("--new-terminal needs a calling herdr-core terminal")
            })?,
        }
    } else {
        if wanted.narrowed {
            return Err(invalid_request(
                "a herdr-core new tab takes --space only; --tab and --pane select Herdr panes",
            ));
        }
        let space = wanted.space.ok_or_else(|| {
            invalid_request("a herdr-core new tab needs --space SPACE (an app space id or label)")
        })?;
        Placement::NewTab {
            // An id or a label: `select_space` resolves it against the core's topology.
            space: HostTargetId::new(space),
        }
    };
    let endpoint = match socket {
        Some(socket) => socket,
        None => discovered(env)?,
    };
    Ok(Some(TargetSelection {
        endpoint,
        placement,
        kind: crate::host::hc::KIND,
    }))
}

/// The one running core, found from state files; none or several is a refusal.
fn discovered(env: EnvLookup<'_>) -> Result<PathBuf, RunError> {
    let mut cores = HcDiscovery::scan(env);
    match cores.len() {
        0 => Err(invalid_request(
            "no herdr-core is running and no Herdr server is selected; run inside a \
             herdr-core terminal or start a core",
        )),
        1 => Ok(cores.remove(0).socket),
        _ => Err(invalid_request(&format!(
            "several herdr-core sessions are running ({}); run inside one of their terminals",
            cores
                .iter()
                .map(|core| core.session.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}
