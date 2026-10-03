//! Evidence-backed native adapter recipe registry.
//!
//! Each harness keeps a table of recipes. A recipe names the exact installed
//! versions it covers — an explicit set, or a closed interval whose every
//! member is known compatible — the evidence that admitted them, what that
//! evidence covers, and a harness-specific profile selecting parse/encode
//! behaviour. Versions are canonical `MAJOR.MINOR.PATCH` decimal triples;
//! anything else (prefixes, suffixes, whitespace, leading zeros, pre-release
//! tags) is unrecognized. [`lookup`] matches only listed versions and never by
//! prefix or nearest recipe; a version it does not list is classified by the
//! admission ladder in [`super::admission`].
use std::fmt;

/// Whether a native capability is supported by the recipe's evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSupport {
    Unsupported,
    Supported,
}

/// Per-harness soft-deadline poke capabilities (spec §10). A recipe declares a
/// capability only from captured evidence; every harness reports `NONE` until
/// such evidence lands, so an undeclared capability means the poke is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PokeCapabilities {
    /// The adapter can stash and later restore the composer's typed text.
    pub composer_stash: NativeSupport,
    /// A prompt submitted during an active turn is queued and safe.
    pub poke_during_turn: NativeSupport,
}

impl PokeCapabilities {
    pub const NONE: Self = Self {
        composer_stash: NativeSupport::Unsupported,
        poke_during_turn: NativeSupport::Unsupported,
    };
}

/// The poke capabilities of the recipe covering `installed` for `harness`.
/// No recipe, an unobserved or unrecognized version, or a harness without
/// recipes (a person) reports `NONE`: an undeclared capability skips the poke.
pub fn poke_capabilities(
    harness: crate::protocol::authority::Harness,
    installed: Option<&str>,
) -> PokeCapabilities {
    use crate::protocol::authority::Harness;
    let Some(installed) = installed else {
        return PokeCapabilities::NONE;
    };
    match harness {
        Harness::Claude => super::claude::recipe_for(installed).map(|recipe| PokeCapabilities {
            composer_stash: recipe.profile.composer_stash,
            poke_during_turn: recipe.profile.poke_during_turn,
        }),
        Harness::Codex => super::codex::recipe_for(installed).map(|recipe| PokeCapabilities {
            composer_stash: recipe.profile.composer_stash,
            poke_during_turn: recipe.profile.poke_during_turn,
        }),
        Harness::Human => return PokeCapabilities::NONE,
    }
    .unwrap_or(PokeCapabilities::NONE)
}

/// A canonical installed-harness version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse exactly `MAJOR.MINOR.PATCH`: ASCII digits only, at most six per
    /// component, no leading zero except a lone `0`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let mut next = || -> Option<u32> {
            let part = parts.next()?;
            if part.is_empty()
                || part.len() > 6
                || !part.bytes().all(|b| b.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
            {
                return None;
            }
            part.parse().ok()
        };
        let version = Self::new(next()?, next()?, next()?);
        if parts.next().is_some() {
            return None;
        }
        Some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The installed versions a recipe covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSet {
    /// Exactly these versions.
    Exact(&'static [Version]),
    /// Every version `v` with `min <= v <= max` (inclusive); a `None` bound
    /// is open. A recipe's own `versions` interval must have both bounds and
    /// be used only when every such version is backed by evidence (for example
    /// consecutive patch releases); an open bound is for `known_broken`.
    Interval {
        min: Option<Version>,
        max: Option<Version>,
    },
}

impl VersionSet {
    pub fn contains(&self, version: Version) -> bool {
        match self {
            Self::Exact(versions) => versions.contains(&version),
            Self::Interval { min, max } => {
                min.is_none_or(|min| min <= version) && max.is_none_or(|max| version <= max)
            }
        }
    }

    /// The least member (an open lower bound reads as `0.0.0`).
    pub fn min_version(&self) -> Version {
        match self {
            Self::Exact(versions) => versions
                .iter()
                .min()
                .copied()
                .unwrap_or(Version::new(0, 0, 0)),
            Self::Interval { min, .. } => min.unwrap_or(Version::new(0, 0, 0)),
        }
    }

    /// The greatest member (an open upper bound reads as the largest version).
    pub fn max_version(&self) -> Version {
        match self {
            Self::Exact(versions) => versions
                .iter()
                .max()
                .copied()
                .unwrap_or(Version::new(0, 0, 0)),
            Self::Interval { max, .. } => max.unwrap_or(Version::new(u32::MAX, u32::MAX, u32::MAX)),
        }
    }
}

impl fmt::Display for VersionSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(versions) => {
                f.write_str("{")?;
                for (index, version) in versions.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{version}")?;
                }
                f.write_str("}")
            }
            Self::Interval {
                min: Some(min),
                max: Some(max),
            } => write!(f, "[{min}, {max}]"),
            Self::Interval {
                min: Some(min),
                max: None,
            } => write!(f, "[{min}, \u{2026})"),
            Self::Interval {
                min: None,
                max: Some(max),
            } => write!(f, "(\u{2026}, {max}]"),
            Self::Interval {
                min: None,
                max: None,
            } => f.write_str("(\u{2026}, \u{2026})"),
        }
    }
}

/// How much native evidence backs one listed version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// A live native run was observed.
    Live,
    /// Input captured (and/or source read), but no live model run.
    NoModel,
    /// No evidence beyond the version being listed.
    None,
}

/// One evidence-backed adapter recipe.
#[derive(Debug, PartialEq, Eq)]
pub struct Recipe<P: 'static> {
    /// Stable recipe name used in diagnostics and docs.
    pub id: &'static str,
    pub versions: VersionSet,
    /// Repository paths of the capture/report evidence admitting `versions`.
    pub evidence: &'static [&'static str],
    /// What the evidence covers, and what it does not.
    pub scope: &'static str,
    /// One entry per listed version: how much evidence backs it. A listed
    /// version without an entry is an error the registry test catches.
    pub evidence_levels: &'static [(Version, Evidence)],
    /// Versions known not to work, each a [`VersionSet::Interval`] (empty =
    /// none known). The admission ladder refuses these before anything else.
    pub known_broken: &'static [VersionSet],
    /// Harness-specific parse/encode behaviour selector and capabilities.
    pub profile: P,
}

impl<P> Recipe<P> {
    /// The least listed version.
    pub fn min_version(&self) -> Version {
        self.versions.min_version()
    }

    /// The greatest listed version.
    pub fn max_version(&self) -> Version {
        self.versions.max_version()
    }

    /// Every listed version: the `Exact` list, or for an `Interval` the
    /// versions of `evidence_levels` (the registry test requires those to
    /// enumerate every patch from min to max).
    pub fn listed_versions(&self) -> Vec<Version> {
        match self.versions {
            VersionSet::Exact(versions) => versions.to_vec(),
            VersionSet::Interval { .. } => self
                .evidence_levels
                .iter()
                .map(|(version, _)| *version)
                .collect(),
        }
    }

    /// The evidence level recorded for `version`, if any.
    pub fn evidence_level(&self, version: Version) -> Option<Evidence> {
        self.evidence_levels
            .iter()
            .find(|(listed, _)| *listed == version)
            .map(|(_, level)| *level)
    }
}

/// Why an installed version string selected no recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    /// Not a canonical `MAJOR.MINOR.PATCH` triple.
    Unrecognized,
    /// Well formed, but outside every recipe.
    Unsupported(Version),
}

/// Select the recipe covering `installed`. Tables must not overlap; the
/// registry tests check that invariant for every production table.
pub fn lookup<'a, P>(
    table: &'a [Recipe<P>],
    installed: &str,
) -> Result<&'a Recipe<P>, LookupError> {
    let version = Version::parse(installed).ok_or(LookupError::Unrecognized)?;
    table
        .iter()
        .find(|recipe| recipe.versions.contains(version))
        .ok_or(LookupError::Unsupported(version))
}

/// `id versions; id versions` for diagnostics, in table order.
pub fn describe<P>(table: &[Recipe<P>]) -> String {
    table
        .iter()
        .map(|recipe| format!("{} {}", recipe.id, recipe.versions))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Actionable refusal for an installed version no recipe covers.
pub fn unsupported_message<P>(harness: &str, installed: &str, table: &[Recipe<P>]) -> String {
    format!(
        "{harness} {installed} has no adapter recipe; supported recipes: {}. \
         Install a supported version, or capture this version's hook payloads \
         and add a recipe backed by that evidence",
        describe(table)
    )
}

/// Actionable refusal for an installed version that could not be observed at
/// all (no binary, a failed or timed-out `--version`, or no observed version
/// supplied). It still names the supported recipes, so an observation failure
/// is as actionable as an uncovered version. `remedy` is the harness-specific
/// next step: only a harness whose witness is an absolute executable the
/// operator configures (Codex) may tell them to point at one.
pub fn unavailable_message<P>(
    harness: &str,
    reason: &str,
    remedy: &str,
    table: &[Recipe<P>],
) -> String {
    format!(
        "{harness} --version could not be observed ({reason}); supported recipes: {}. {remedy}",
        describe(table)
    )
}

/// Actionable refusal for any lookup failure: an unrecognized version string
/// and an uncovered version both name the supported recipes, so every
/// production refusal tells the operator which versions would work.
pub fn refusal_message<P>(
    harness: &str,
    installed: &str,
    table: &[Recipe<P>],
    error: &LookupError,
) -> String {
    match error {
        LookupError::Unrecognized => format!(
            "{harness} --version output is not a canonical X.Y.Z version; supported recipes: {}",
            describe(table)
        ),
        LookupError::Unsupported(_) => unsupported_message(harness, installed, table),
    }
}
