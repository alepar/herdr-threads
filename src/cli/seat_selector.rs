//! Pure seat-selector resolver (spec D3, D6 renderer, D2 step 2 prefix check).
//!
//! Owns: the readings table (whole string plus the `/` split), per-level matching (space label or
//! ID; tab label or ID; pane label, live agent name or pane ID), the sole-pane-tab and
//! sole-tab-space aliases, narrowing by already-resolved space/tab IDs, pooling and dedupe by pane,
//! tier ranking, the nickname-prefix and narrowed-location checks, the `space/tab/pane` path
//! renderer, and the bounded not_found / conflict text with its single `ApiError` conversion.
//!
//! Does not: read the host, call the daemon, know which command it serves, allocate or look up
//! seats itself (the caller passes a fallible lookup closure), or print. New reading or matching
//! behaviour goes into `readings` and `reading_matches`, not into callers.

use crate::cli::panes::{looks_like_pane_id, looks_like_tab_id};
use crate::host::observation::{HostTopology, TopologyPane, TopologySpace, TopologyTab};
use crate::protocol::{
    ids::{HostTargetId, SeatId},
    results::ApiError,
};
use crate::view::escape::{Context, escape_for_terminal, is_format_char};
use std::fmt;

/// Safety cap on rendered conflict candidates (spec D3); beyond it, "N omitted".
pub const CONFLICT_CAP: usize = 32;
/// Longest label rendered as itself (spec D6 renderer contract).
pub const SAFE_LABEL_MAX_BYTES: usize = 64;
/// Longest user-supplied text echoed into an error line.
const ECHO_MAX_BYTES: usize = 64;
const QUALIFY_HINT: &str =
    "qualify with a longer SPACE/TAB/PANE path, --space/--tab, or an exact seat or pane ID";

/// Already-resolved `--space`/`--tab` narrowing (host IDs, never labels).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Narrowing {
    pub space: Option<String>,
    pub tab: Option<String>,
}

impl Narrowing {
    fn admits(&self, pane: &TopologyPane) -> bool {
        self.space.as_ref().is_none_or(|s| pane.space == *s)
            && self.tab.as_ref().is_none_or(|t| pane.tab == *t)
    }
}

/// Distance from the caller's live pane; earlier variants win.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Tab,
    Space,
    Any,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Tab => "tab",
            Tier::Space => "space",
            Tier::Any => "any",
        }
    }
}

/// Which levels a reading assigned segments to. `Display` renders `space=A tab=B pane=C`,
/// omitting unassigned levels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub space: Option<String>,
    pub tab: Option<String>,
    pub pane: Option<String>,
}

impl fmt::Display for Reading {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (name, value) in [
            ("space", &self.space),
            ("tab", &self.tab),
            ("pane", &self.pane),
        ] {
            if let Some(value) = value {
                if !first {
                    f.write_str(" ")?;
                }
                first = false;
                write!(f, "{name}={}", echo(value))?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub pane: HostTargetId,
    pub tier: Tier,
    /// Deduplicated, in readings-table order.
    pub readings: Vec<Reading>,
    /// Full live `space/tab/pane` path (D6 renderer).
    pub path: String,
}

/// Result of resolving one selector string against one topology.
///
/// This module owns readings, matching, aliases, narrowing, pooling, tiers, the nickname and
/// location checks, the path renderer and the bounded error text. It never reads the host, calls
/// the daemon, allocates or looks up seats, or prints; the runtime stage supplies the topology,
/// the caller pane and a seat-lookup closure. New reading or matching behaviour is added here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Resolved(Candidate),
    Ambiguous(Vec<Candidate>),
    NoMatch,
}

/// Location check for a seat whose mapped target is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationCheck {
    Match,
    Mismatch { live_path: String },
    Unverifiable,
}

/// User-supplied text for an error line: escaped to one line and cut to a bounded size.
fn echo(text: &str) -> String {
    let escaped = escape_for_terminal(text, Context::SingleLine);
    if escaped.len() <= ECHO_MAX_BYTES {
        return escaped.into_owned();
    }
    let mut end = ECHO_MAX_BYTES;
    while !escaped.is_char_boundary(end) {
        end -= 1;
    }
    escaped[..end].to_owned()
}

// ---------------------------------------------------------------- readings and matching

fn reading(space: Option<&str>, tab: Option<&str>, pane: Option<&str>) -> Reading {
    let own = |s: &str| s.to_owned();
    Reading {
        space: space.map(own),
        tab: tab.map(own),
        pane: pane.map(own),
    }
}

/// The readings table (spec D3), in a fixed order: the whole string as one segment, then the
/// `/` split when it has two or three non-empty segments.
fn readings(selector: &str) -> Vec<Reading> {
    let mut out = vec![
        reading(None, None, Some(selector)),
        reading(None, Some(selector), None),
        reading(Some(selector), None, None),
    ];
    let segs: Vec<&str> = selector.split('/').collect();
    if segs.iter().any(|s| s.is_empty()) {
        return out;
    }
    match segs[..] {
        [a, b] => {
            out.push(reading(Some(a), Some(b), None));
            out.push(reading(None, Some(a), Some(b)));
            out.push(reading(Some(a), None, Some(b)));
        }
        [a, b, c] => out.push(reading(Some(a), Some(b), Some(c))),
        _ => {}
    }
    out
}

fn space_of<'a>(topology: &'a HostTopology, id: &str) -> Option<&'a TopologySpace> {
    topology.spaces.iter().find(|s| s.id == id)
}

fn tab_of<'a>(topology: &'a HostTopology, id: &str) -> Option<&'a TopologyTab> {
    topology.tabs.iter().find(|t| t.id == id)
}

fn tab_has_one_pane(topology: &HostTopology, pane: &TopologyPane) -> bool {
    topology.panes.iter().filter(|p| p.tab == pane.tab).count() == 1
}

fn space_has_one_tab_with_one_pane(topology: &HostTopology, pane: &TopologyPane) -> bool {
    topology
        .tabs
        .iter()
        .filter(|t| t.space == pane.space)
        .count()
        == 1
        && topology
            .panes
            .iter()
            .filter(|p| p.space == pane.space)
            .count()
            == 1
}

fn reading_matches(topology: &HostTopology, pane: &TopologyPane, reading: &Reading) -> bool {
    if let Some(seg) = &reading.space {
        let hit = pane.space == *seg
            || space_of(topology, &pane.space).is_some_and(|s| s.label.as_deref() == Some(seg));
        if !hit {
            return false;
        }
    }
    if let Some(seg) = &reading.tab {
        let hit = pane.tab == *seg
            || tab_of(topology, &pane.tab).is_some_and(|t| t.label.as_deref() == Some(seg));
        if !hit {
            return false;
        }
    }
    if let Some(seg) = &reading.pane {
        let hit = pane.label.as_deref() == Some(seg)
            || pane.agent_names.iter().any(|name| name == seg)
            || pane.target.as_str() == seg;
        if !hit {
            return false;
        }
    }
    // Aliases: a tab named without a pane needs exactly one pane; a space named alone needs
    // exactly one tab with exactly one pane.
    match (&reading.space, &reading.tab, &reading.pane) {
        (None, Some(_), None) | (Some(_), Some(_), None) => tab_has_one_pane(topology, pane),
        (Some(_), None, None) => space_has_one_tab_with_one_pane(topology, pane),
        _ => true,
    }
}

/// Every admitted pane (topology order) matched by at least one reading, with its readings.
fn pool<'a>(
    selector: &str,
    topology: &'a HostTopology,
    narrowing: &Narrowing,
) -> Vec<(&'a TopologyPane, Vec<Reading>)> {
    let all = readings(selector);
    topology
        .panes
        .iter()
        .filter(|pane| narrowing.admits(pane))
        .filter_map(|pane| {
            let mut hits: Vec<Reading> = Vec::new();
            for reading in &all {
                if reading_matches(topology, pane, reading) && !hits.contains(reading) {
                    hits.push(reading.clone());
                }
            }
            (!hits.is_empty()).then_some((pane, hits))
        })
        .collect()
}

fn tier_of(topology: &HostTopology, caller: Option<&HostTargetId>, pane: &TopologyPane) -> Tier {
    let Some(caller) = caller.and_then(|c| topology.panes.iter().find(|p| p.target == *c)) else {
        return Tier::Any;
    };
    if caller.tab == pane.tab {
        Tier::Tab
    } else if caller.space == pane.space {
        Tier::Space
    } else {
        Tier::Any
    }
}

/// Resolve `selector` (spec D3): pool every reading, narrow, rank by tier, take the first
/// non-empty tier.
pub fn resolve(
    selector: &str,
    topology: &HostTopology,
    caller: Option<&HostTargetId>,
    narrowing: &Narrowing,
) -> Outcome {
    let mut tiered: Vec<Candidate> = pool(selector, topology, narrowing)
        .into_iter()
        .map(|(pane, readings)| Candidate {
            pane: pane.target.clone(),
            tier: tier_of(topology, caller, pane),
            readings,
            path: render_pane(topology, pane),
        })
        .collect();
    let Some(best) = tiered.iter().map(|c| c.tier).min() else {
        return Outcome::NoMatch;
    };
    tiered.retain(|c| c.tier == best);
    if tiered.len() == 1 {
        Outcome::Resolved(tiered.remove(0))
    } else {
        Outcome::Ambiguous(tiered)
    }
}

// ---------------------------------------------------------------- nickname and location

/// `Some((prefix, last))` when `selector` contains `/` and both the prefix and the last segment
/// are non-empty (split at the LAST `/`). The caller decides whether `last` is an existing seat.
pub fn split_nickname(selector: &str) -> Option<(&str, &str)> {
    let (prefix, last) = selector.rsplit_once('/')?;
    (!prefix.is_empty() && !last.is_empty()).then_some((prefix, last))
}

fn pane_of<'a>(topology: &'a HostTopology, target: &HostTargetId) -> Option<&'a TopologyPane> {
    topology.panes.iter().find(|p| p.target == *target)
}

/// Spec D2 step 2: is the seat's mapped target among the panes the prefix names (all tiers
/// pooled, no narrowing)?
pub fn verify_nickname(
    prefix: &str,
    topology: &HostTopology,
    mapped_target: Option<&HostTargetId>,
) -> LocationCheck {
    let Some(pane) = mapped_target.and_then(|t| pane_of(topology, t)) else {
        return LocationCheck::Unverifiable;
    };
    let inside = pool(prefix, topology, &Narrowing::default())
        .iter()
        .any(|(p, _)| p.target == pane.target);
    location_result(inside, topology, pane)
}

/// Spec D2 "Narrowing with an exact ID or nickname": the target must lie inside the narrowed
/// space (and tab, if given).
pub fn verify_location(
    topology: &HostTopology,
    narrowing: &Narrowing,
    mapped_target: Option<&HostTargetId>,
) -> LocationCheck {
    let Some(pane) = mapped_target.and_then(|t| pane_of(topology, t)) else {
        return LocationCheck::Unverifiable;
    };
    location_result(narrowing.admits(pane), topology, pane)
}

fn location_result(inside: bool, topology: &HostTopology, pane: &TopologyPane) -> LocationCheck {
    if inside {
        LocationCheck::Match
    } else {
        LocationCheck::Mismatch {
            live_path: render_pane(topology, pane),
        }
    }
}

impl LocationCheck {
    /// `Match` is `Ok`; the other two are the D2 `Conflict` texts naming the live path or
    /// asking for the bare seat ID.
    pub fn into_result(self, selector: &str, seat: &SeatId) -> Result<(), ApiError> {
        let seat = seat.as_str();
        match self {
            LocationCheck::Match => Ok(()),
            LocationCheck::Mismatch { live_path } => Err(ApiError::conflict(format!(
                "seat {seat} is at {live_path}, not {}; pass the bare seat ID {seat}",
                echo(selector),
            ))),
            LocationCheck::Unverifiable => Err(ApiError::conflict(format!(
                "cannot verify location of {seat}; pass the bare seat ID"
            ))),
        }
    }
}

// ---------------------------------------------------------------- renderer

fn plain_safe(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= SAFE_LABEL_MAX_BYTES
        && !label.contains('/')
        && !label
            .chars()
            .any(|c| c.is_control() || is_format_char(c) || c.is_whitespace())
        && !looks_like_pane_id(label)
        && !looks_like_tab_id(label)
}

fn space_part(topology: &HostTopology, pane: &TopologyPane) -> String {
    let id = pane.space.as_str();
    match space_of(topology, id).and_then(|s| s.label.as_deref()) {
        Some(label)
            if plain_safe(label)
                && !topology.spaces.iter().any(|o| {
                    o.id != id && (o.label.as_deref() == Some(label) || o.id == label)
                }) =>
        {
            label.to_owned()
        }
        _ => id.to_owned(),
    }
}

fn tab_part(topology: &HostTopology, pane: &TopologyPane) -> String {
    let id = pane.tab.as_str();
    match tab_of(topology, id).and_then(|t| t.label.as_deref()) {
        Some(label)
            if plain_safe(label)
                && !topology.tabs.iter().any(|o| {
                    o.space == pane.space
                        && o.id != id
                        && (o.label.as_deref() == Some(label) || o.id == label)
                }) =>
        {
            label.to_owned()
        }
        _ => id.to_owned(),
    }
}

fn pane_part(topology: &HostTopology, pane: &TopologyPane) -> String {
    match pane.label.as_deref() {
        Some(label)
            if plain_safe(label)
                && !topology.panes.iter().any(|o| {
                    o.tab == pane.tab
                        && o.target != pane.target
                        && (o.label.as_deref() == Some(label)
                            || o.agent_names.iter().any(|n| n == label)
                            || o.target.as_str() == label)
                }) =>
        {
            label.to_owned()
        }
        _ => pane.target.as_str().to_owned(),
    }
}

fn render_pane(topology: &HostTopology, pane: &TopologyPane) -> String {
    format!(
        "{}/{}/{}",
        space_part(topology, pane),
        tab_part(topology, pane),
        pane_part(topology, pane)
    )
}

/// The full live `space/tab/pane` path of `pane` (spec D6 renderer contract); `None` when the
/// pane is not in the topology.
pub fn render_path(topology: &HostTopology, pane: &HostTargetId) -> Option<String> {
    pane_of(topology, pane).map(|p| render_pane(topology, p))
}

// ---------------------------------------------------------------- error text

impl Outcome {
    /// The single conversion to `ApiError` (spec D8). `seat_of` runs only for rendered
    /// candidates (at most [`CONFLICT_CAP`]); `Err` renders `seat=?`, `Ok(None)` `seat=none`.
    pub fn into_result<E>(
        self,
        selector: &str,
        mut seat_of: impl FnMut(&HostTargetId) -> Result<Option<SeatId>, E>,
    ) -> Result<Candidate, ApiError> {
        match self {
            Outcome::Resolved(candidate) => Ok(candidate),
            Outcome::NoMatch => Err(ApiError::not_found(format!(
                "seat {}: neither an existing seat ID nor a live space/tab/pane name",
                echo(selector)
            ))),
            Outcome::Ambiguous(candidates) => {
                let tier = candidates.first().map_or(Tier::Any, |c| c.tier);
                let mut lines = vec![format!(
                    "ambiguous seat {}: {} candidates (tier: {})",
                    echo(selector),
                    candidates.len(),
                    tier.as_str()
                )];
                for candidate in candidates.iter().take(CONFLICT_CAP) {
                    let readings = candidate
                        .readings
                        .iter()
                        .map(|r| r.to_string())
                        .collect::<Vec<_>>()
                        .join("; ");
                    let seat = match seat_of(&candidate.pane) {
                        Ok(Some(id)) => id.as_str().to_owned(),
                        Ok(None) => "none".to_owned(),
                        Err(_) => "?".to_owned(),
                    };
                    lines.push(format!(
                        "  {readings} {} {} seat={seat}",
                        candidate.path,
                        candidate.pane.as_str()
                    ));
                }
                if candidates.len() > CONFLICT_CAP {
                    lines.push(format!("  ({} omitted)", candidates.len() - CONFLICT_CAP));
                }
                lines.push(QUALIFY_HINT.to_owned());
                Err(ApiError::conflict(lines.join("\n")))
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/cli/seat_selector.rs"]
mod tests;
