//! Command-local scoped human locators, frozen to canonical pane IDs before requests.
//! Labels and live agent names are exact lookup aids, never seat continuity evidence.

use super::commands::{CliAction, MutationSpec, ParsedCli};
use crate::protocol::{ids::HostTargetId, results::ApiError};

/// Command-local exact target locator. Parsed separately from canonical wire types.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct PaneSelector {
    /// Exact workspace ID or label; omitted inherits the caller's live workspace, never focus.
    #[arg(long)]
    pub space: Option<String>,
    /// Exact tab ID or label in the selected workspace; a tab ID supplies its workspace.
    #[arg(long)]
    pub tab: Option<String>,
    /// Exact pane ID, label or live agent name in the selected tab. Omitted parents
    /// inherit the live caller; outside Herdr supply parents or a pane ID.
    /// Qualify duplicate names with --space/--tab or use an ID; conflicts never pick a pane.
    #[arg(long)]
    pub pane: Option<String>,
}

impl PaneSelector {
    pub fn is_explicit(&self) -> bool {
        self.space.is_some() || self.tab.is_some() || self.pane.is_some()
    }
    pub fn direct_id(&self) -> Option<HostTargetId> {
        self.pane
            .as_ref()
            .filter(|pane| looks_like_pane_id(pane) && self.space.is_none() && self.tab.is_none())
            .and_then(|pane| HostTargetId::parse(pane).ok())
    }
}

fn unique<'a, T>(
    matches: Vec<&'a T>,
    kind: &str,
    describe: impl Fn(&T) -> String,
) -> Result<&'a T, ApiError> {
    match matches.as_slice() {
        [one] => Ok(*one),
        [] => Err(ApiError::not_found(format!(
            "no matching {kind}; {NAME_PRECEDENCE}; {FIND_PANE_ID_HINT}"
        ))),
        many => Err(ApiError::new(
            crate::protocol::results::ErrorCode::Conflict,
            format!(
                "ambiguous {kind}: {} matches ({} omitted): {}; qualify with --space SPACE --tab TAB --pane PANE; {FIND_PANE_ID_HINT}",
                many.len(),
                many.len().saturating_sub(8),
                many.iter()
                    .take(8)
                    .map(|item| describe(item))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// Resolve exact IDs before names against one snapshot and a live caller ID.
/// This pure locator never reads or allocates a durable seat.
pub fn resolve_selector(
    selector: &PaneSelector,
    topology: &crate::host::observation::HostTopology,
    caller: Option<&str>,
) -> Result<HostTargetId, ApiError> {
    let caller = caller.and_then(|id| {
        topology
            .panes
            .iter()
            .find(|pane| pane.target.as_str() == id)
    });
    let exact = selector.pane.as_ref().and_then(|id| {
        topology
            .panes
            .iter()
            .find(|pane| pane.target.as_str() == id)
    });
    if selector
        .pane
        .as_ref()
        .is_some_and(|value| looks_like_pane_id(value))
        && exact.is_none()
    {
        return Err(ApiError::not_found(
            "the qualified pane ID is absent from the live topology",
        ));
    }
    let explicit_tab_id = selector
        .tab
        .as_ref()
        .and_then(|id| topology.tabs.iter().find(|tab| &tab.id == id));
    if selector
        .tab
        .as_ref()
        .is_some_and(|value| looks_like_tab_id(value))
        && explicit_tab_id.is_none()
    {
        return Err(ApiError::not_found(
            "the explicit tab ID is absent from the live topology",
        ));
    }
    let space = if let Some(value) = &selector.space {
        topology
            .spaces
            .iter()
            .find(|space| &space.id == value)
            .map(Ok)
            .unwrap_or_else(|| {
                unique(
                    topology
                        .spaces
                        .iter()
                        .filter(|space| space.label.as_ref() == Some(value))
                        .collect(),
                    "space",
                    |space| format!("{:?} {:?}", space.id, space.label),
                )
            })?
    } else {
        let id = exact.map(|pane| &pane.space).or_else(|| explicit_tab_id.map(|tab| &tab.space))
            .or_else(|| caller.map(|pane| &pane.space)).ok_or_else(|| invalid(
                "outside Herdr, pass --space/--tab or a pane ID; caller pane is absent or stale".into()))?;
        topology
            .spaces
            .iter()
            .find(|space| &space.id == id)
            .ok_or_else(|| ApiError::not_found("caller space disappeared"))?
    };
    let tab = if let Some(value) = &selector.tab {
        if let Some(tab) = explicit_tab_id {
            if tab.space != space.id {
                return Err(invalid(
                    "explicit tab does not belong to selected space".into(),
                ));
            }
            tab
        } else {
            unique(
                topology
                    .tabs
                    .iter()
                    .filter(|tab| tab.space == space.id && tab.label.as_ref() == Some(value))
                    .collect(),
                "tab",
                |tab| format!("{:?} {:?}", tab.id, tab.label),
            )?
        }
    } else if let Some(pane) = exact {
        topology
            .tabs
            .iter()
            .find(|tab| tab.id == pane.tab && tab.space == space.id)
            .ok_or_else(|| invalid("explicit pane does not belong to selected space".into()))?
    } else if let Some(pane) = caller.filter(|pane| pane.space == space.id) {
        topology
            .tabs
            .iter()
            .find(|tab| tab.id == pane.tab)
            .ok_or_else(|| ApiError::not_found("caller tab disappeared"))?
    } else {
        unique(
            topology
                .tabs
                .iter()
                .filter(|tab| tab.space == space.id)
                .collect(),
            "tab",
            |tab| format!("{:?} {:?}", tab.id, tab.label),
        )?
    };
    let scoped: Vec<_> = topology
        .panes
        .iter()
        .filter(|pane| pane.space == space.id && pane.tab == tab.id)
        .collect();
    let describe = |pane: &crate::host::observation::TopologyPane| {
        let kind = match selector.pane.as_ref() {
            Some(value) => match (
                pane.label.as_ref() == Some(value),
                pane.agent_names.contains(value),
            ) {
                (true, true) => "pane label and live agent name",
                (true, false) => "pane label",
                (false, true) => "live agent name",
                _ => "single-pane tab alias",
            },
            None => "child pane",
        };
        format!(
            "space {:?} {:?}, tab {:?} {:?}, pane {:?} {:?} ({kind})",
            space.id,
            space.label,
            tab.id,
            tab.label,
            pane.target.as_str(),
            pane.label
        )
    };
    if let Some(pane) = exact {
        if pane.space != space.id || pane.tab != tab.id {
            return Err(invalid(
                "explicit pane does not belong to selected parents".into(),
            ));
        }
        return Ok(pane.target.clone());
    }
    let matches = match &selector.pane {
        Some(value) => {
            let matches: Vec<_> = scoped
                .iter()
                .copied()
                .filter(|pane| {
                    pane.label.as_ref() == Some(value) || pane.agent_names.contains(value)
                })
                .collect();
            if matches.is_empty() && scoped.len() == 1 && tab.label.as_ref() == Some(value) {
                scoped
            } else {
                matches
            }
        }
        None => {
            if let Some(pane) = caller.filter(|pane| pane.space == space.id && pane.tab == tab.id) {
                return Ok(pane.target.clone());
            }
            scoped
        }
    };
    let kind = selector
        .pane
        .as_ref()
        .map(|value| format!("pane {value:?}"))
        .unwrap_or_else(|| "pane".into());
    unique(matches, &kind, describe).map(|pane| pane.target.clone())
}

/// How a person finds a pane ID; part of every name-resolution error.
pub const FIND_PANE_ID_HINT: &str =
    "run `herdr pane current` inside that pane (or `herdr pane list`) to get its pane ID";

/// Exact locator matching and scope, included in missing-pane diagnostics.
pub const NAME_PRECEDENCE: &str = "an exact pane ID wins; pane label and live agent name match together within the selected tab; a tab label is an alias only when that tab holds exactly one pane";

/// Tab IDs have an unambiguous separator/prefix, unlike workspace labels such as `work`.
fn looks_like_tab_id(value: &str) -> bool {
    value.split_once(':').is_some_and(|(space, tab)| {
        space.len() > 1
            && space.starts_with('w')
            && space[1..].bytes().all(|byte| byte.is_ascii_alphanumeric())
            && tab.len() > 1
            && tab.starts_with('t')
            && tab[1..].bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

/// The Herdr 0.9.1 pane ID shape `w<workspace>:p<pane>`. Such a value is used
/// as given, exactly as before names were accepted.
pub fn looks_like_pane_id(value: &str) -> bool {
    let Some((workspace, pane)) = value.split_once(':') else {
        return false;
    };
    let part = |text: &str, prefix: char| {
        text.len() > 1
            && text.starts_with(prefix)
            && text[1..].bytes().all(|b| b.is_ascii_alphanumeric())
    };
    part(workspace, 'w') && part(pane, 'p')
}

fn invalid(detail: String) -> ApiError {
    ApiError::invalid_request(detail)
}

#[cfg(test)]
#[path = "../../tests/cli/panes_hint.rs"]
mod panes_hint;

/// Freeze all CLI target locators once, leaving canonical IDs for journaling.
/// Explicit unqualified pane IDs retain their direct compatibility path.
pub fn resolve_cli_targets(
    parsed: &mut ParsedCli,
    topology: impl FnOnce() -> Result<crate::host::observation::HostTopology, ApiError>,
    live_caller: impl FnOnce() -> Result<Option<HostTargetId>, ApiError>,
) -> Result<(), ApiError> {
    let cooperative = parsed.cooperative_selector.clone();
    let selectors: Vec<_> = parsed
        .pane_selector
        .iter()
        .chain(parsed.require_ack_panes.iter())
        .chain(cooperative.iter())
        .collect();
    let needs_topology = selectors
        .iter()
        .any(|selector| selector.direct_id().is_none());
    let topology = if needs_topology {
        Some(topology()?)
    } else {
        None
    };
    let needs_caller = selectors.iter().any(|selector| {
        // Even fully specified parents need the live caller when the pane is
        // omitted: the same tab defaults to own pane, a different tab to its
        // sole child. An explicit pane has no such same-parent default.
        if selector.pane.is_none() {
            return true;
        }
        let pane_id = selector
            .pane
            .as_ref()
            .is_some_and(|pane| looks_like_pane_id(pane));
        let qualified = selector.space.is_some() && selector.tab.is_some();
        let tab_id = selector.tab.as_ref().is_some_and(|id| {
            looks_like_tab_id(id)
                || topology
                    .as_ref()
                    .is_some_and(|topology| topology.tabs.iter().any(|tab| &tab.id == id))
        });
        !(pane_id || qualified || tab_id)
    });
    let caller = if needs_caller { live_caller()? } else { None };
    let resolve = |selector: &PaneSelector| -> Result<HostTargetId, ApiError> {
        match selector.direct_id() {
            Some(id) => Ok(id),
            None => resolve_selector(
                selector,
                topology.as_ref().expect("topology requested"),
                caller.as_ref().map(HostTargetId::as_str),
            ),
        }
    };
    if let Some(selector) = &parsed.pane_selector {
        let target = resolve(selector)?;
        match &mut parsed.action {
            CliAction::Mutation(
                MutationSpec::Resolve(pane)
                | MutationSpec::FreshSeat(pane)
                | MutationSpec::Rebind { pane, .. }
                | MutationSpec::Replace { pane, .. },
            ) => *pane = target.clone(),
            CliAction::Launch(request) => request.target = target.clone(),
            CliAction::Handoff(request) => request.launch.target = target.clone(),
            _ => {}
        }
        parsed.pane_selector = Some(PaneSelector {
            pane: Some(target.as_str().to_owned()),
            ..Default::default()
        });
    }
    for selector in &mut parsed.require_ack_panes {
        *selector = PaneSelector {
            pane: Some(resolve(selector)?.as_str().to_owned()),
            ..Default::default()
        };
    }
    let mut seen = std::collections::HashSet::new();
    parsed
        .require_ack_panes
        .retain(|selector| seen.insert(selector.pane.clone()));
    if let (Some(selector), Some(selection)) = (cooperative, &mut parsed.cooperative) {
        selection.target = resolve(&selector)?;
        parsed.cooperative_selector = Some(PaneSelector {
            pane: Some(selection.target.as_str().to_owned()),
            ..Default::default()
        });
    }
    Ok(())
}
