//! The installer-managed installation the running executable belongs to, and the state
//! directories `scripts/install.sh` scans (the same two Herdr plugin state roots).
use crate::daemon::paths::{DATABASE_FILE, node_dir};
use std::path::{Path, PathBuf};

/// The installer's marker file inside the installation directory.
pub const MARKER: &str = ".herdr-threads-install";
const PLUGIN_ID: &str = "herdr-threads";

/// `<prefix>/.herdr-threads-install`, one `key=value` per line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marker {
    pub version: Option<String>,
    pub sha256: Option<String>,
    pub bin_dir: Option<PathBuf>,
    /// `herdr=0`: installed with `--no-herdr`.
    pub no_herdr: bool,
    /// `permissions=0`: installed with `--without-permissions`.
    pub without_permissions: bool,
    /// `aisw_profiles=0`: installed with `--no-aisw-profiles`.
    pub no_aisw_profiles: bool,
}

impl Marker {
    pub fn parse(text: &str) -> Self {
        let mut marker = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "version" => marker.version = Some(value.to_owned()),
                "sha256" => marker.sha256 = Some(value.to_owned()),
                "bin_dir" if value.starts_with('/') => marker.bin_dir = Some(value.into()),
                "herdr" => marker.no_herdr = value == "0",
                "permissions" => marker.without_permissions = value == "0",
                "aisw_profiles" => marker.no_aisw_profiles = value == "0",
                _ => {}
            }
        }
        marker
    }

    pub fn read(prefix: &Path) -> std::io::Result<Self> {
        Ok(Self::parse(&std::fs::read_to_string(prefix.join(MARKER))?))
    }
}

/// An installer-managed installation: `<prefix>/bin/herdr-threads` plus its marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    /// The physical installation directory (`P`).
    pub prefix: PathBuf,
    pub marker: Marker,
}

impl Installation {
    /// The installation `exe` (physically resolved) belongs to, if any.
    pub fn of_executable(exe: &Path) -> Option<Self> {
        let exe = std::fs::canonicalize(exe).ok()?;
        let bin = exe.parent()?;
        if exe.file_name()? != "herdr-threads" || bin.file_name()? != "bin" {
            return None;
        }
        let prefix = bin.parent()?.to_path_buf();
        let marker = Marker::read(&prefix).ok()?;
        Some(Self { prefix, marker })
    }

    /// The installation of this process's executable. Test builds may name the executable
    /// with `HT_TEST_UPDATE_EXE` (a packaged wrapper that execs the test build).
    pub fn current() -> Option<Self> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(exe) = std::env::var_os("HT_TEST_UPDATE_EXE").filter(|exe| !exe.is_empty()) {
            return Self::of_executable(Path::new(&exe));
        }
        Self::of_executable(&std::env::current_exe().ok()?)
    }

    pub fn binary(&self) -> PathBuf {
        self.prefix.join("bin").join("herdr-threads")
    }

    /// `P<suffix>`, a sibling of the installation directory (`P.prev`, `P.update.json`).
    pub fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = self.prefix.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    }

    /// The directory of the `herdr-threads` and `ht` links: the marker's, else the
    /// installer's default `~/.local/bin`.
    pub fn bin_dir(&self) -> Option<PathBuf> {
        self.marker.bin_dir.clone().or_else(|| {
            std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .map(|home| PathBuf::from(home).join(".local").join("bin"))
        })
    }

    /// Whether a daemon started from `exe` belongs to this installation: run from `P` or
    /// `P.old` (the installer's `owned_daemon` rule). A deleted file is judged by its path.
    pub fn owns_executable(&self, exe: &Path) -> bool {
        let exe = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
        exe.starts_with(&self.prefix) || exe.starts_with(self.sibling(".old"))
    }
}

/// The two Herdr plugin state roots the installer scans, existing directories only,
/// deduplicated by physical path.
pub fn default_state_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    let xdg = std::env::var_os("XDG_STATE_HOME")
        .filter(|xdg| !xdg.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".local").join("state")));
    let mut dirs: Vec<PathBuf> = Vec::new();
    for base in [xdg, home.map(|home| home.join(".local").join("state"))]
        .into_iter()
        .flatten()
    {
        let dir = base.join("herdr").join("plugins").join(PLUGIN_ID);
        if let Ok(physical) = std::fs::canonicalize(&dir)
            && physical.is_dir()
            && !dirs.contains(&physical)
        {
            dirs.push(physical);
        }
    }
    dirs
}

/// The default state directories holding a node store (the installer's `store_states`).
pub fn store_state_dirs() -> Vec<PathBuf> {
    default_state_dirs()
        .into_iter()
        .filter(|dir| {
            let store = node_dir(dir).join(DATABASE_FILE);
            store.exists() || store.is_symlink()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_lines_parse_opt_outs_and_ignore_unknown_keys() {
        let marker = Marker::parse(
            "version=2.7.0\nsha256=ab\nasset=x\nbin_dir=/opt/bin\nherdr=0\npermissions=0\nfuture=1\n",
        );
        assert_eq!(marker.version.as_deref(), Some("2.7.0"));
        assert_eq!(marker.bin_dir.as_deref(), Some(Path::new("/opt/bin")));
        assert!(marker.no_herdr && marker.without_permissions && !marker.no_aisw_profiles);
        let old = Marker::parse("version=2.6.3\nsha256=ab\nasset=x\n");
        assert_eq!(old.bin_dir, None);
        assert!(!old.no_herdr);
        assert_eq!(Marker::parse("bin_dir=relative\n").bin_dir, None);
    }
}
