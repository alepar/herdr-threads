//! Pure seat-selector resolver, shared by the CLI and the daemon (remote addresses) (spec D3, D6 renderer, D2 step 2 prefix check).
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
    InvalidPath,
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

/// One parse shared by name and nickname resolution. Raw readings always keep their original
/// meaning; decoding is opt-in and never changes the separate location flags.
pub struct Selector<'a> {
    raw: &'a str,
    components: Option<Result<Vec<String>, ()>>,
}

fn path_components(path: &str) -> Result<Vec<String>, ()> {
    let mut parts = vec![String::new()];
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        match c {
            '/' => parts.push(String::new()),
            '\\' => match chars.next() {
                Some(c @ ('/' | '\\' | '@')) => parts.last_mut().unwrap().push(c),
                _ => return Err(()),
            },
            _ => parts.last_mut().unwrap().push(c),
        }
    }
    if parts.iter().any(String::is_empty) || parts.len() > 4 {
        return Err(());
    }
    Ok(parts)
}

fn component_readings(parts: &[String]) -> Vec<Reading> {
    match parts {
        [a] => vec![
            reading(None, None, Some(a)),
            reading(None, Some(a), None),
            reading(Some(a), None, None),
        ],
        [a, b] => vec![
            reading(Some(a), Some(b), None),
            reading(None, Some(a), Some(b)),
            reading(Some(a), None, Some(b)),
        ],
        [a, b, c] => vec![reading(Some(a), Some(b), Some(c))],
        _ => Vec::new(),
    }
}

/// A possible nickname, with its prefix already parsed into level readings.
pub struct Nickname<'a> {
    pub last: &'a str,
    prefix: Vec<Reading>,
    needs_raw_scope: bool,
}

impl Nickname<'_> {
    pub fn is_legacy_path(&self) -> bool {
        self.needs_raw_scope
    }

    /// An opt-in argument retains an existing raw nickname scope before its decoded scope.
    pub fn applies(&self, topology: &HostTopology) -> bool {
        !self.needs_raw_scope
            || !pool_readings(&self.prefix, topology, &Narrowing::default()).is_empty()
    }

    pub fn verify(&self, topology: &HostTopology, target: Option<&HostTargetId>) -> LocationCheck {
        let Some(pane) = target.and_then(|t| pane_of(topology, t)) else {
            return LocationCheck::Unverifiable;
        };
        location_result(
            self.prefix
                .iter()
                .any(|r| reading_matches(topology, pane, r)),
            topology,
            pane,
        )
    }

    pub fn conflict_with(
        &self,
        other: &Nickname<'_>,
        topology: &HostTopology,
        target: Option<&HostTargetId>,
        other_target: Option<&HostTargetId>,
    ) -> Option<Outcome> {
        let pane = target.and_then(|t| pane_of(topology, t))?;
        let other_pane = other_target.and_then(|t| pane_of(topology, t))?;
        if pane.target == other_pane.target {
            return None;
        }
        Some(Outcome::Ambiguous(
            [(self, pane), (other, other_pane)]
                .into_iter()
                .map(|(nickname, pane)| Candidate {
                    pane: pane.target.clone(),
                    tier: Tier::Any,
                    readings: nickname.prefix.clone(),
                    path: render_pane(topology, pane),
                })
                .collect(),
        ))
    }
}

impl<'a> Selector<'a> {
    pub fn new(raw: &'a str) -> Self {
        Self {
            raw,
            components: raw.strip_prefix("path:").map(path_components),
        }
    }

    pub fn nicknames(&self) -> Vec<Nickname<'_>> {
        let mut out = Vec::new();
        if let Some((prefix, last)) = split_nickname(self.raw) {
            out.push(Nickname {
                last,
                prefix: readings(prefix),
                needs_raw_scope: self.components.is_some(),
            });
        }
        if let Some(Ok(parts)) = &self.components
            && parts.len() >= 2
        {
            out.push(Nickname {
                last: parts.last().unwrap(),
                prefix: component_readings(&parts[..parts.len() - 1]),
                needs_raw_scope: false,
            });
        }
        out
    }

    fn decoded_readings(&self) -> Result<Vec<Reading>, ()> {
        match &self.components {
            None => Ok(Vec::new()),
            Some(Ok(parts)) if parts.len() <= 3 => Ok(component_readings(parts)),
            _ => Err(()),
        }
    }

    fn candidates(
        &self,
        topology: &HostTopology,
        caller: Option<&HostTargetId>,
        narrowing: &Narrowing,
        include_raw: bool,
    ) -> (Vec<Candidate>, bool) {
        let raw = if include_raw {
            readings(self.raw)
        } else {
            Vec::new()
        };
        let decoded = self.decoded_readings().unwrap_or_default();
        let mut raw_hit = false;
        let mut decoded_hit = false;
        let candidates = topology
            .panes
            .iter()
            .filter(|pane| narrowing.admits(pane))
            .filter_map(|pane| {
                let mut hits = Vec::new();
                for (all, matched) in [(&raw, &mut raw_hit), (&decoded, &mut decoded_hit)] {
                    for reading in all {
                        if reading_matches(topology, pane, reading) {
                            *matched = true;
                            if !hits.contains(reading) {
                                hits.push(reading.clone());
                            }
                        }
                    }
                }
                (!hits.is_empty()).then(|| Candidate {
                    pane: pane.target.clone(),
                    tier: tier_of(topology, caller, pane),
                    readings: hits,
                    path: render_pane(topology, pane),
                })
            })
            .collect();
        (candidates, raw_hit && decoded_hit)
    }

    pub fn resolve_with(
        &self,
        topology: &HostTopology,
        caller: Option<&HostTargetId>,
        narrowing: &Narrowing,
        tiering: Tiering,
    ) -> Outcome {
        if self.raw.is_empty() {
            return Outcome::NoMatch;
        }
        let (mut candidates, both) = self.candidates(topology, caller, narrowing, true);
        if both && candidates.len() > 1 {
            return Outcome::Ambiguous(candidates);
        }
        let Some(best) = candidates.iter().map(|c| c.tier).min() else {
            return if self.decoded_readings().is_err() {
                Outcome::InvalidPath
            } else {
                Outcome::NoMatch
            };
        };
        if tiering == Tiering::Proximity {
            candidates.retain(|c| c.tier == best);
        }
        if candidates.len() == 1 {
            Outcome::Resolved(candidates.remove(0))
        } else {
            Outcome::Ambiguous(candidates)
        }
    }

    /// A nickname is an existing seat, but must not hide another raw or decoded full-name
    /// target. Return all contenders without proximity filtering; narrowing still applies.
    pub fn nickname_collision(
        &self,
        nickname: &Nickname<'_>,
        topology: &HostTopology,
        narrowing: &Narrowing,
        target: Option<&HostTargetId>,
    ) -> Option<Outcome> {
        self.components.as_ref()?;
        let pane = target.and_then(|t| pane_of(topology, t))?;
        // A winning raw nickname historically outranks raw full-name matches. Only a decoded
        // target can conflict with it; a decoded nickname must also check raw full names.
        let (mut candidates, _) =
            self.candidates(topology, None, narrowing, !nickname.is_legacy_path());
        if !candidates.iter().any(|c| c.pane != pane.target) {
            return None;
        }
        if !candidates.iter().any(|c| c.pane == pane.target) {
            candidates.push(Candidate {
                pane: pane.target.clone(),
                tier: Tier::Any,
                readings: nickname.prefix.clone(),
                path: render_pane(topology, pane),
            });
        }
        Some(Outcome::Ambiguous(candidates))
    }
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
    pool_readings(&all, topology, narrowing)
}

fn pool_readings<'a>(
    all: &[Reading],
    topology: &'a HostTopology,
    narrowing: &Narrowing,
) -> Vec<(&'a TopologyPane, Vec<Reading>)> {
    topology
        .panes
        .iter()
        .filter(|pane| narrowing.admits(pane))
        .filter_map(|pane| {
            let mut hits: Vec<Reading> = Vec::new();
            for reading in all {
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

/// How candidates are ranked. `Proximity` is spec D3's rule for every ordinary seat argument;
/// `Global` is for operator seat repair (`seat retire`/`rebind`/`--replace`), which never lets a
/// nearer pane win: any second match anywhere is a conflict, so an irreversible repair acts
/// only on a name that is unique in the whole topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tiering {
    Proximity,
    Global,
}

/// Resolve `selector` (spec D3): pool every reading, narrow, rank by tier, take the first
/// non-empty tier.
pub fn resolve(
    selector: &str,
    topology: &HostTopology,
    caller: Option<&HostTargetId>,
    narrowing: &Narrowing,
) -> Outcome {
    resolve_with(selector, topology, caller, narrowing, Tiering::Proximity)
}

/// [`resolve`] with an explicit [`Tiering`]. An empty selector never matches (not even an empty
/// label).
pub fn resolve_with(
    selector: &str,
    topology: &HostTopology,
    caller: Option<&HostTargetId>,
    narrowing: &Narrowing,
    tiering: Tiering,
) -> Outcome {
    Selector::new(selector).resolve_with(topology, caller, narrowing, tiering)
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
            Outcome::InvalidPath => Err(ApiError::invalid_request(
                r"invalid path: use 1-3 nonempty components; escapes: \/, \\ and \@",
            )),
            Outcome::NoMatch => Err(ApiError::not_found(format!(
                "seat {}: neither an existing seat ID nor a live space/tab/pane name",
                echo(selector)
            ))),
            Outcome::Ambiguous(candidates) => {
                let first = candidates.first().map_or(Tier::Any, |c| c.tier);
                // Global tiering (seat repair) keeps candidates from every tier.
                let tier = if candidates.iter().all(|c| c.tier == first) {
                    first.as_str()
                } else {
                    "mixed"
                };
                let mut lines = vec![format!(
                    "ambiguous seat {}: {} candidates (tier: {tier})",
                    echo(selector),
                    candidates.len(),
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

/// One `--space/--tab/--pane` selector as a single reading, pooled over the
/// whole topology (no caller pane: remote addresses, spec D2 "Parent
/// inheritance on M"). Matching and aliases are [`reading_matches`]'s.
pub fn resolve_parts(
    topology: &HostTopology,
    space: Option<&str>,
    tab: Option<&str>,
    pane: Option<&str>,
) -> Outcome {
    if space.is_none() && tab.is_none() && pane.is_none() {
        return Outcome::NoMatch;
    }
    let wanted = reading(space, tab, pane);
    let mut candidates: Vec<Candidate> = topology
        .panes
        .iter()
        .filter(|p| reading_matches(topology, p, &wanted))
        .map(|p| Candidate {
            pane: p.target.clone(),
            tier: Tier::Any,
            readings: vec![wanted.clone()],
            path: render_pane(topology, p),
        })
        .collect();
    match candidates.len() {
        0 => Outcome::NoMatch,
        1 => Outcome::Resolved(candidates.remove(0)),
        _ => Outcome::Ambiguous(candidates),
    }
}

/// The `@MACHINE` split of an address (remote addresses, spec D2):
/// `(location, machine)` at the last `@` whose machine part is nonempty and
/// holds no `/` or `\\`, with a nonempty location. In the `path:` form a
/// `\\@` is a literal `@`, never the separator. The location keeps its
/// `path:` prefix and its escapes. `None`: the address names no machine.
pub fn split_machine(raw: &str) -> Option<(&str, &str)> {
    let at = match raw.strip_prefix("path:") {
        Some(path) => {
            let mut last = None;
            let mut chars = path.char_indices();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => {
                        chars.next();
                    }
                    '@' => last = Some("path:".len() + i),
                    _ => {}
                }
            }
            last?
        }
        None => raw.rfind('@')?,
    };
    let (location, machine) = (&raw[..at], &raw[at + 1..]);
    let bare = location.strip_prefix("path:").unwrap_or(location);
    (!bare.is_empty() && !machine.is_empty() && !machine.contains(['/', '\\']))
        .then_some((location, machine))
}

/// Whether `raw` (an `@MACHINE` suffix) names the node `row` (spec D1): its
/// full uuid or a prefix of at least 8 hex digits (any case), its exact tag,
/// or its host label or that label's first DNS label, ignoring ASCII case.
pub fn machine_matches(raw: &str, row: &crate::protocol::sync_admin::MachineRow) -> bool {
    let hex = (8..=32).contains(&raw.len()) && raw.bytes().all(|b| b.is_ascii_hexdigit());
    (hex && row.uuid.starts_with(&raw.to_ascii_lowercase()))
        || raw == row.tag
        || row.host.as_deref().is_some_and(|host| {
            host.eq_ignore_ascii_case(raw)
                || host
                    .split('.')
                    .next()
                    .is_some_and(|first| first.eq_ignore_ascii_case(raw))
        })
}

/// Every row `raw` names, each node once, in the rows' order.
pub fn match_machine<'a>(
    raw: &str,
    rows: &'a [crate::protocol::sync_admin::MachineRow],
) -> Vec<&'a crate::protocol::sync_admin::MachineRow> {
    let mut out: Vec<&crate::protocol::sync_admin::MachineRow> = Vec::new();
    for row in rows.iter().filter(|row| machine_matches(raw, row)) {
        if !out.iter().any(|seen| seen.uuid == row.uuid) {
            out.push(row);
        }
    }
    out
}

/// Tab IDs have an unambiguous separator/prefix, unlike workspace labels such as `work`.
pub fn looks_like_tab_id(value: &str) -> bool {
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

#[cfg(test)]
#[path = "../../tests/cli/seat_selector.rs"]
mod tests;
