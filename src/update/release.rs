//! Release versions, the GitHub releases API document, `SHA256SUMS` lines and the bounded
//! `curl` fetch used by `update` and the update check.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// The releases API of the repository `scripts/install.sh` installs from.
pub const DEFAULT_API: &str = "https://api.github.com/repos/alepar/herdr-threads/releases";
/// Overrides [`DEFAULT_API`] (a mirror, or a `file://` tree in tests): `<base>/latest` and
/// `<base>/tags/<tag>` must serve the API's release documents.
pub const API_ENV: &str = "HERDR_THREADS_RELEASE_API";
/// Bound for an API document request.
pub const API_TIMEOUT_SECS: u64 = 10;
/// Bound for an archive download.
pub const DOWNLOAD_TIMEOUT_SECS: u64 = 300;
/// The largest API document read.
const MAX_DOCUMENT_BYTES: u64 = 1 << 20;

/// A release version `X.Y.Z` (a leading `v` is accepted and dropped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');
        let mut next = || -> Option<u64> {
            let part = parts.next()?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let version = Self(next()?, next()?, next()?);
        parts.next().is_none().then_some(version)
    }

    /// This executable's own version.
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("the package version is X.Y.Z")
    }

    pub fn tag(self) -> String {
        format!("v{self}")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// `<os>-<arch>` as the release assets and the archive's `TARGET` name it.
pub fn platform() -> Result<String, String> {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        other => return Err(format!("unsupported operating system: {other}")),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => return Err(format!("unsupported architecture: {other}")),
    };
    Ok(format!("{os}-{arch}"))
}

pub fn asset_name(platform: &str) -> String {
    format!("herdr-threads-{platform}.tar.gz")
}

pub const SUMS_ASSET: &str = "SHA256SUMS";

/// One published release, as the releases API describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    /// Lower-case hex SHA-256 from the API's `digest` (`sha256:<hex>`), when present.
    pub sha256: Option<String>,
    pub url: String,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    #[serde(default)]
    digest: Option<String>,
    browser_download_url: String,
}

/// Parse a releases API document. Drafts, prereleases and tags that are not `vX.Y.Z` are
/// refused.
pub fn parse_release(bytes: &[u8]) -> Result<Release, String> {
    let api: ApiRelease = serde_json::from_slice(bytes)
        .map_err(|error| format!("the release document is not valid: {error}"))?;
    if api.draft || api.prerelease {
        return Err(format!("{} is a draft or prerelease", api.tag_name));
    }
    let version = Version::parse(&api.tag_name)
        .filter(|_| api.tag_name.starts_with('v'))
        .ok_or_else(|| format!("release tag {:?} is not vX.Y.Z", api.tag_name))?;
    let assets = api
        .assets
        .into_iter()
        .map(|asset| Asset {
            sha256: asset
                .digest
                .as_deref()
                .and_then(|digest| digest.strip_prefix("sha256:"))
                .filter(|hex| is_sha256_hex(hex))
                .map(str::to_ascii_lowercase),
            name: asset.name,
            url: asset.browser_download_url,
        })
        .collect();
    Ok(Release { version, assets })
}

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The `SHA256SUMS` hash of `asset` (`<hex>  <name>` or `<hex> *<name>`).
pub fn sums_entry(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hex = fields.next()?;
        let name = fields.next()?;
        let name = name.strip_prefix('*').unwrap_or(name);
        (name == asset && is_sha256_hex(hex)).then(|| hex.to_ascii_lowercase())
    })
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// The releases API base: [`API_ENV`] when set, else [`DEFAULT_API`].
pub fn api_base() -> String {
    std::env::var(API_ENV)
        .ok()
        .filter(|base| !base.is_empty())
        .unwrap_or_else(|| DEFAULT_API.to_owned())
        .trim_end_matches('/')
        .to_owned()
}

/// The release document URL for `version`, or for the latest release.
pub fn release_url(base: &str, version: Option<Version>) -> String {
    match version {
        Some(version) => format!("{base}/tags/{}", version.tag()),
        None => format!("{base}/latest"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// No `curl` on `PATH`.
    NoCurl,
    Failed(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCurl => f.write_str("curl is not on PATH"),
            Self::Failed(detail) => f.write_str(detail),
        }
    }
}

fn curl() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("curl"))
        .find(|candidate| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

/// Download `url` to `out` with `curl`, bounded by `timeout_secs` (curl's `--max-time`).
pub fn download(url: &str, out: &Path, timeout_secs: u64) -> Result<(), FetchError> {
    fetch_to(url, out, timeout_secs, false)
}

fn fetch_to(url: &str, out: &Path, timeout_secs: u64, api: bool) -> Result<(), FetchError> {
    let curl = curl().ok_or(FetchError::NoCurl)?;
    let mut command = Command::new(curl);
    command
        // No retries: --max-time then bounds the whole operation.
        .args(["-fsSL", "--connect-timeout", "10", "--max-time"])
        .arg(timeout_secs.to_string());
    if api {
        command.args(["-H", "Accept: application/vnd.github+json"]);
    }
    let output = command
        .arg("-o")
        .arg(out)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| FetchError::Failed(format!("curl did not run: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.lines().last().unwrap_or("").trim();
    Err(FetchError::Failed(format!(
        "download failed: {url}{}{detail}",
        if detail.is_empty() { "" } else { ": " }
    )))
}

/// Fetch and parse the release document for `version` (latest when `None`) into `work`.
pub fn fetch_release(
    base: &str,
    version: Option<Version>,
    work: &Path,
    timeout_secs: u64,
) -> Result<Release, FetchError> {
    let path = work.join(format!("release-{}.json", uuid::Uuid::new_v4()));
    let result = fetch_to(&release_url(base, version), &path, timeout_secs, true).and_then(|()| {
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .and_then(|file| file.take(MAX_DOCUMENT_BYTES).read_to_end(&mut bytes))
            .map_err(|error| FetchError::Failed(format!("release document unreadable: {error}")))?;
        parse_release(&bytes).map_err(FetchError::Failed)
    });
    let _ = std::fs::remove_file(&path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_order_numerically() {
        assert_eq!(Version::parse("v2.10.0"), Some(Version(2, 10, 0)));
        assert_eq!(Version::parse("2.6.3"), Some(Version(2, 6, 3)));
        assert!(Version::parse("2.6").is_none());
        assert!(Version::parse("2.6.3.1").is_none());
        assert!(Version::parse("2.6.x").is_none());
        assert!(Version::parse("2.6.3-rc1").is_none());
        assert!(Version(2, 10, 0) > Version(2, 9, 9));
        assert_eq!(Version(2, 7, 0).tag(), "v2.7.0");
    }

    #[test]
    fn release_documents_parse_digests_and_refuse_drafts() {
        let doc = br#"{"tag_name":"v2.7.0","draft":false,"prerelease":false,"assets":[
            {"name":"SHA256SUMS","digest":"sha256:AB00000000000000000000000000000000000000000000000000000000000000","browser_download_url":"file:///s"},
            {"name":"herdr-threads-linux-x86_64.tar.gz","digest":null,"browser_download_url":"file:///a"}]}"#;
        let release = parse_release(doc).unwrap();
        assert_eq!(release.version, Version(2, 7, 0));
        assert_eq!(
            release.asset(SUMS_ASSET).unwrap().sha256.as_deref(),
            Some("ab00000000000000000000000000000000000000000000000000000000000000")
        );
        assert_eq!(
            release
                .asset("herdr-threads-linux-x86_64.tar.gz")
                .unwrap()
                .sha256,
            None
        );
        assert!(parse_release(br#"{"tag_name":"v2.7.0","draft":true}"#).is_err());
        assert!(parse_release(br#"{"tag_name":"v2.7.0","prerelease":true}"#).is_err());
        assert!(parse_release(br#"{"tag_name":"2.7.0"}"#).is_err());
        assert!(parse_release(b"not json").is_err());
    }

    #[test]
    fn sums_lines_match_the_exact_asset() {
        let hex = "0".repeat(63) + "1";
        let sums = format!(
            "{hex}  herdr-threads-linux-x86_64.tar.gz\n{}  other\n",
            "f".repeat(64)
        );
        assert_eq!(
            sums_entry(&sums, "herdr-threads-linux-x86_64.tar.gz").as_deref(),
            Some(hex.as_str())
        );
        assert_eq!(
            sums_entry(&format!("{hex} *a.tar.gz\n"), "a.tar.gz").as_deref(),
            Some(hex.as_str())
        );
        assert_eq!(sums_entry(&sums, "herdr-threads-linux"), None);
        assert_eq!(sums_entry("xyz  a.tar.gz\n", "a.tar.gz"), None);
    }

    #[test]
    fn release_urls() {
        assert_eq!(release_url("b", None), "b/latest");
        assert_eq!(release_url("b", Some(Version(2, 7, 1))), "b/tags/v2.7.1");
    }
}
