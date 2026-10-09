//! Herdr release policy: a semver floor, not an allowlist.
//!
//! Herdr's own plugin contract is `min_herdr_version` in the manifest, and the
//! adapter enforces the same floor at connect time. Every release at or above
//! it is admitted; the JSON API is then checked per operation (an unknown
//! method or rejected params is `Unsupported` for that operation only) and
//! optional extensions still negotiate their exact capability.
//!
//! `protocol` is Herdr's private binary-protocol number. It moved 14 -> 22
//! across 0.7.0 - 0.9.0 and 0.9.2 broke the JSON API without changing it, so
//! it is recorded for diagnostics and never gates the connection.

use std::cmp::Ordering;
use std::fmt;

/// Equal to `min_herdr_version` in `herdr-plugin.toml` (a test keeps them in
/// step). Raise both together, and only when Threads starts depending on a
/// newer Herdr API.
pub const MIN_HERDR_VERSION: Version = Version::new(0, 9, 1);

/// The newest release Threads was qualified against: the newest version in
/// `tests/herdr-releases.tsv`, which `scripts/herdr-release-smoke` runs the
/// e2e smoke against (a test keeps them in step). A newer release is admitted
/// with one Health warning.
pub const NEWEST_TESTED_HERDR_VERSION: Version = Version::new(0, 9, 3);

/// Releases known to break Threads although they are above the floor. Empty
/// today; an entry refuses that exact release (prereleases included).
const DENIED_RELEASES: &[Version] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// A `-pre` suffix orders below the same release (semver), so `0.9.1-rc1`
    /// is below the floor.
    pub prerelease: bool,
}

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            prerelease: false,
        }
    }

    /// Parses a strict SemVer 2.0.0 string: `MAJOR.MINOR.PATCH` without
    /// leading zeroes, then optional `-PRERELEASE` and `+BUILD`, each a
    /// nonempty dot-separated list of nonempty `[0-9A-Za-z-]` identifiers
    /// (numeric prerelease identifiers without leading zeroes). Anything else,
    /// a leading `v` or surrounding whitespace included, is `None`: Herdr
    /// reports its Cargo package version, which is always valid semver.
    pub fn parse(text: &str) -> Option<Self> {
        let (text, build) = match text.split_once('+') {
            Some((text, build)) => (text, Some(build)),
            None => (text, None),
        };
        if build.is_some_and(|build| !identifiers(build, false)) {
            return None;
        }
        let (core, prerelease) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        if prerelease.is_some_and(|pre| !identifiers(pre, true)) {
            return None;
        }
        let mut parts = core.split('.');
        let mut number = || parts.next().and_then(numeric);
        let (major, minor, patch) = (number()?, number()?, number()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
            prerelease: prerelease.is_some(),
        })
    }

    fn release(self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }
}

/// A semver numeric identifier: digits, no leading zero unless exactly `0`.
fn numeric(part: &str) -> Option<u64> {
    let digits = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    (digits && (part == "0" || !part.starts_with('0')))
        .then(|| part.parse().ok())
        .flatten()
}

/// Dot-separated nonempty `[0-9A-Za-z-]` identifiers; with `prerelease`, an
/// all-digit identifier must also be a valid numeric identifier.
fn identifiers(list: &str, prerelease: bool) -> bool {
    list.split('.').all(|id| {
        let charset = !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        let all_digits = id.bytes().all(|b| b.is_ascii_digit());
        charset && !(prerelease && all_digits && numeric(id).is_none())
    })
}

/// Orders by release, a prerelease below its release. Prereleases of the
/// same release compare equal: the floor and tested range are stable
/// releases, so their relative order is never needed.
impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.release()
            .cmp(&other.release())
            .then_with(|| other.prerelease.cmp(&self.prerelease))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What one `ping` reported, kept for Health and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRelease {
    /// The reported version string, bounded.
    pub version: String,
    /// The private binary-protocol number, diagnostics only.
    pub protocol: Option<u64>,
    pub support: ReleaseSupport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseSupport {
    /// Within the qualified range.
    Tested,
    /// Above the newest qualified release: admitted, with a Health warning.
    Untested,
}

impl HostRelease {
    /// One line for `HostHealth.version`: the version plus the protocol.
    pub fn summary(&self) -> String {
        match self.protocol {
            Some(protocol) => format!("{} (protocol {protocol})", self.version),
            None => self.version.clone(),
        }
    }

    /// The Health warning for an untested newer release, if this is one.
    pub fn warning(&self) -> Option<String> {
        (self.support == ReleaseSupport::Untested).then(|| {
            format!(
                "untested Herdr {}; tested {MIN_HERDR_VERSION}-{NEWEST_TESTED_HERDR_VERSION}",
                self.version
            )
        })
    }
}

/// Admits a `ping` answer by version floor alone. The error is the refusal
/// detail. `protocol` is recorded, never compared.
pub fn admit(version: Option<&str>, protocol: Option<u64>) -> Result<HostRelease, String> {
    let reported = version.ok_or_else(|| "Herdr did not report its version".to_owned())?;
    let shown: String = reported.chars().take(64).collect();
    let parsed = Version::parse(reported)
        .ok_or_else(|| format!("Herdr reported an unparseable version {shown:?}"))?;
    if parsed < MIN_HERDR_VERSION {
        return Err(format!(
            "Herdr {shown} is older than the minimum {MIN_HERDR_VERSION}; upgrade Herdr"
        ));
    }
    if DENIED_RELEASES.contains(&Version {
        prerelease: false,
        ..parsed
    }) {
        return Err(format!("Herdr {shown} is known to break Threads"));
    }
    let support = if parsed.release() > NEWEST_TESTED_HERDR_VERSION.release() {
        ReleaseSupport::Untested
    } else {
        ReleaseSupport::Tested
    };
    Ok(HostRelease {
        version: shown,
        protocol,
        support,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_matches_the_manifest() {
        let manifest: toml_edit::DocumentMut =
            include_str!("../../herdr-plugin.toml").parse().unwrap();
        let floor = manifest["min_herdr_version"].as_str().unwrap();
        assert_eq!(Version::parse(floor), Some(MIN_HERDR_VERSION));
        assert_eq!(MIN_HERDR_VERSION.to_string(), floor);
        assert!(NEWEST_TESTED_HERDR_VERSION >= MIN_HERDR_VERSION);
        // The release smoke's own floor filter (`--update` keeps releases at
        // or above it) must be the same floor.
        let floor = MIN_HERDR_VERSION;
        let smoke_floor = format!(
            "FLOOR = ({}, {}, {})",
            floor.major, floor.minor, floor.patch
        );
        assert!(
            include_str!("../../scripts/herdr-release-smoke").contains(&smoke_floor),
            "scripts/herdr-release-smoke must declare {smoke_floor}"
        );
    }

    /// The tested range is exactly what the release smoke covers: its oldest
    /// and newest listed releases. Adding a release to the list (by hand or
    /// `herdr-release-smoke --update`) fails here until the constant moves.
    #[test]
    fn tested_range_matches_the_release_smoke_list() {
        let listed: Vec<Version> = include_str!("../../tests/herdr-releases.tsv")
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .map(|line| {
                let version = line.split('\t').next().unwrap();
                Version::parse(version).unwrap_or_else(|| panic!("bad version {version:?}"))
            })
            .collect();
        assert_eq!(listed.iter().max(), Some(&NEWEST_TESTED_HERDR_VERSION));
        assert_eq!(listed.iter().min(), Some(&MIN_HERDR_VERSION));
    }

    #[test]
    fn parses_semver_forms() {
        assert_eq!(Version::parse("0.9.2"), Some(Version::new(0, 9, 2)));
        assert_eq!(Version::parse("1.10.0"), Some(Version::new(1, 10, 0)));
        for valid in [
            "0.9.4+abc",
            "0.9.4+build.007",
            "0.9.4+exp.sha.5114f85",
            "10.20.30",
        ] {
            assert!(
                Version::parse(valid).is_some_and(|v| !v.prerelease),
                "{valid}"
            );
        }
        for prerelease in [
            "0.9.4-rc.1",
            "0.9.4-alpha-1",
            "0.9.4-0",
            "0.9.4-x.7.z.92+b.1",
        ] {
            assert!(
                Version::parse(prerelease).is_some_and(|v| v.prerelease),
                "{prerelease}"
            );
        }
        for bad in [
            "",
            "0.9",
            "0.9.1.2",
            "0.9.x",
            "0.9.-1",
            "0.9.1-",
            "x",
            // Malformed build metadata.
            "0.9.1+",
            "0.9.1+bad+build",
            "0.9.1+a..b",
            "0.9.1+a_b",
            // Leading zeroes in the core or a numeric prerelease identifier.
            "00.9.1",
            "0.09.1",
            "0.9.01",
            "0.9.2-01",
            // Malformed prerelease identifiers.
            "0.9.2-!",
            "0.9.2-rc..1",
            "0.9.2-rc.",
            "0.9.2-+b",
            // Not semver: a `v` prefix or surrounding whitespace.
            "v0.9.1",
            " 0.9.1",
            "0.9.1\n",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert!(Version::parse("0.9.1-rc1").unwrap() < MIN_HERDR_VERSION);
        assert!(Version::parse("0.10.0").unwrap() > Version::new(0, 9, 3));
    }

    #[test]
    fn admits_every_release_from_the_floor_regardless_of_protocol() {
        for version in ["0.9.1", "0.9.2", "0.9.3"] {
            for protocol in [Some(22), Some(23), Some(14), None] {
                let release = admit(Some(version), protocol).unwrap();
                assert_eq!(release.support, ReleaseSupport::Tested, "{version}");
                assert_eq!(release.warning(), None);
            }
        }
    }

    #[test]
    fn newer_releases_connect_with_one_warning() {
        for version in ["0.9.4", "0.10.0", "1.0.0", "0.9.4-rc.1"] {
            let release = admit(Some(version), Some(23)).unwrap();
            assert_eq!(release.support, ReleaseSupport::Untested, "{version}");
        }
        let release = admit(Some("0.10.0"), Some(23)).unwrap();
        assert_eq!(
            release.warning().as_deref(),
            Some("untested Herdr 0.10.0; tested 0.9.1-0.9.3")
        );
        assert_eq!(release.summary(), "0.10.0 (protocol 23)");
    }

    #[test]
    fn refuses_below_the_floor_and_unreadable_versions() {
        for version in ["0.9.0", "0.8.2", "0.9.1-rc1", "0.7.5"] {
            let refusal = admit(Some(version), Some(22)).unwrap_err();
            assert!(
                refusal.contains("older than the minimum 0.9.1"),
                "{refusal}"
            );
        }
        assert!(admit(None, Some(22)).is_err());
        assert!(admit(Some("dev"), Some(22)).is_err());
        // Main review: malformed semver must not be admitted (three of these
        // were once classified Tested).
        for malformed in ["0.9.1+", "0.9.1+bad+build", "00.9.1", "0.9.2-!"] {
            let refusal = admit(Some(malformed), Some(22)).unwrap_err();
            assert!(
                refusal.contains("unparseable version"),
                "{malformed}: {refusal}"
            );
        }
    }
}
