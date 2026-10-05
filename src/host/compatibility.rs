//! Audited Herdr JSON API releases, independently of optional capabilities.

/// Protocol equality alone does not qualify a release: Herdr changed its
/// machine API and lifecycle behavior without changing protocol 22.
/// This gate admits the audited base contract only. Optional operations must
/// separately negotiate their exact capability against the serving host.
pub(crate) fn supports_json_api(version: Option<&str>, protocol: Option<u64>) -> bool {
    matches!(version, Some("0.9.1" | "0.9.3")) && protocol == Some(22)
}
