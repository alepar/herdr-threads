//! Evidence-backed native adapter recipe registry.
//!
//! Each harness keeps a table of recipes. A recipe names the exact installed
//! versions it covers — an explicit set, or a closed interval whose every
//! member is known compatible — the evidence that admitted them, what that
//! evidence covers, and a harness-specific profile selecting parse/encode
//! behaviour. Versions are canonical `MAJOR.MINOR.PATCH` decimal triples;
//! anything else (prefixes, suffixes, whitespace, leading zeros, pre-release
//! tags) is unrecognized. A well-formed version outside every recipe is
//! rejected with a message naming the supported recipes: unknown versions are
//! fail-closed, never matched by prefix or by nearest recipe.
use std::fmt;

/// Whether a native capability is supported by the recipe's evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSupport {
    Unsupported,
    Supported,
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
    /// Every version `v` with `min <= v <= max`. Use only when every such
    /// version is backed by evidence (for example consecutive patch releases).
    Interval { min: Version, max: Version },
}

impl VersionSet {
    pub fn contains(&self, version: Version) -> bool {
        match self {
            Self::Exact(versions) => versions.contains(&version),
            Self::Interval { min, max } => *min <= version && version <= *max,
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
            Self::Interval { min, max } => write!(f, "[{min}, {max}]"),
        }
    }
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
    /// Harness-specific parse/encode behaviour selector and capabilities.
    pub profile: P,
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
