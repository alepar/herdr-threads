//! Profile-scoped owned assets. Installed/configured are never native activation.
use super::runtime::ProfileObservation;
use crate::harness::setup::{SetupError, fingerprint, write_replacement};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
const MAX: u64 = 1_048_576;
const NAMES: [&str; 3] = ["__init__.py", "bridge_config.json", "plugin.yaml"];
const LOCK: &str = ".herdr-threads-operation.lock";
const LOCK_IDENTITY: &str = ".herdr-threads-operation.identity";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetStatus {
    pub installed: bool,
    pub repairable: bool,
    /// Effective-config enable lists only, never discovered/activated.
    pub configured_enabled: Option<bool>,
    pub manual_argv: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub residue: Vec<String>,
    pub manual_argv: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    digest: String,
    prior: Option<String>,
    bytes: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    home: PathBuf,
    installation_token: String,
    phase: String,
    lock_inode: (u64, u64),
    directory_inode: Option<(u64, u64)>,
    stage_name: Option<String>,
    assets: BTreeMap<String, Asset>,
}
fn guidance(profile: &str, action: &str) -> Vec<String> {
    vec![
        "hermes".into(),
        "--profile".into(),
        profile.into(),
        "plugins".into(),
        action.into(),
        "herdr-threads".into(),
    ]
}
fn valid_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control))
}
fn identity(meta: &fs::Metadata) -> (u64, u64) {
    (meta.dev(), meta.ino())
}
fn regular_dir(path: &Path, create: bool) -> Result<(), SetupError> {
    if create {
        crate::daemon::paths::ensure_owned_state_root(path).map_err(|_| SetupError::Invalid)?;
    } else {
        crate::daemon::paths::check_owned_state_root(path).map_err(|_| SetupError::Invalid)?;
    }
    Ok(())
}
struct ProfileLock {
    file: File,
    home: PathBuf,
    plugins: PathBuf,
    home_inode: (u64, u64),
    plugins_inode: (u64, u64),
    marker_inode: (u64, u64),
}
impl ProfileLock {
    fn acquire(home: &Path, exclusive: bool, create: bool) -> Result<Self, SetupError> {
        if !valid_path(home) || home.canonicalize().map_err(|_| SetupError::Invalid)? != home {
            return Err(SetupError::Invalid);
        }
        regular_dir(home, false)?;
        let home_inode = identity(&fs::symlink_metadata(home).map_err(|_| SetupError::Io)?);
        let plugins = home.join("plugins");
        regular_dir(&plugins, create)?;
        let plugins_inode = identity(&fs::symlink_metadata(&plugins).map_err(|_| SetupError::Io)?);
        let lock_path = plugins.join(LOCK);
        // A separate persistent marker remembers the original inode even after
        // exact removal. Lost/replaced locks are never silently recreated.
        if matches!(fs::symlink_metadata(&lock_path),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
            && (fs::symlink_metadata(plugins.join(LOCK_IDENTITY)).is_ok()
                || fs::symlink_metadata(plugins.join("herdr-threads")).is_ok())
        {
            return Err(SetupError::Conflict);
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(plugins.join(LOCK))
            .map_err(|_| SetupError::Conflict)?;
        let meta = file.metadata().map_err(|_| SetupError::Io)?;
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o077 != 0
            || meta.nlink() != 1
        {
            return Err(SetupError::Conflict);
        }
        let operation = if exclusive {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        if unsafe { libc::flock(file.as_raw_fd(), operation | libc::LOCK_NB) } != 0 {
            return Err(SetupError::Conflict);
        }
        let marker_inode = lock_identity_marker(&plugins, identity(&meta), create)?;
        let lock = Self {
            file,
            home: home.into(),
            plugins,
            home_inode,
            plugins_inode,
            marker_inode,
        };
        lock.validate()?;
        Ok(lock)
    }
    fn validate(&self) -> Result<(), SetupError> {
        let home = fs::symlink_metadata(&self.home).map_err(|_| SetupError::Conflict)?;
        let plugins = fs::symlink_metadata(&self.plugins).map_err(|_| SetupError::Conflict)?;
        let named =
            fs::symlink_metadata(self.plugins.join(LOCK)).map_err(|_| SetupError::Conflict)?;
        let held = self.file.metadata().map_err(|_| SetupError::Conflict)?;
        let marker = fs::symlink_metadata(self.plugins.join(LOCK_IDENTITY))
            .map_err(|_| SetupError::Conflict)?;
        if !marker.is_file() || identity(&marker) != self.marker_inode {
            return Err(SetupError::Conflict);
        }

        if !home.is_dir()
            || !plugins.is_dir()
            || !named.is_file()
            || identity(&home) != self.home_inode
            || identity(&plugins) != self.plugins_inode
            || identity(&named) != identity(&held)
            || held.nlink() != 1
        {
            return Err(SetupError::Conflict);
        }
        Ok(())
    }
}
fn lock_identity_marker(
    plugins: &Path,
    expected: (u64, u64),
    create: bool,
) -> Result<(u64, u64), SetupError> {
    let path = plugins.join(LOCK_IDENTITY);
    match read_optional(&path)? {
        Some(bytes) => {
            if bytes.len() > 128
                || serde_json::from_slice::<(u64, u64)>(&bytes).ok() != Some(expected)
            {
                return Err(SetupError::Conflict);
            }
        }
        None if create => {
            if fs::symlink_metadata(plugins.join("herdr-threads")).is_ok() {
                return Err(SetupError::Conflict);
            }
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)
                .map_err(|_| SetupError::Conflict)?;
            file.write_all(&serde_json::to_vec(&expected).map_err(|_| SetupError::Invalid)?)
                .and_then(|_| file.sync_all())
                .map_err(|_| SetupError::Io)?;
            File::open(plugins)
                .and_then(|f| f.sync_all())
                .map_err(|_| SetupError::Io)?;
        }
        None => return Err(SetupError::Conflict),
    }
    let meta = fs::symlink_metadata(path).map_err(|_| SetupError::Conflict)?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.nlink() != 1
    {
        return Err(SetupError::Conflict);
    }
    Ok(identity(&meta))
}

fn manifest_path(state: &Path, home: &Path, create: bool) -> Result<PathBuf, SetupError> {
    if !valid_path(state) {
        return Err(SetupError::Invalid);
    }
    if create {
        regular_dir(state, true)?;
        crate::daemon::paths::ensure_private_dir(&state.join("setup"))
            .map_err(|_| SetupError::Invalid)?;
        crate::daemon::paths::ensure_private_dir(&state.join("setup/hermes"))
            .map_err(|_| SetupError::Invalid)?;
    } else {
        for directory in [
            state.to_path_buf(),
            state.join("setup"),
            state.join("setup/hermes"),
        ] {
            if fs::symlink_metadata(&directory).is_ok() {
                regular_dir(&directory, false)?;
            }
        }
    }
    Ok(state.join("setup/hermes").join(format!(
        "{}.json",
        fingerprint(home.as_os_str().as_encoded_bytes())
    )))
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, SetupError> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(SetupError::Conflict),
    };
    let meta = file.metadata().map_err(|_| SetupError::Io)?;
    if !meta.is_file() || meta.len() > MAX {
        return Err(SetupError::Conflict);
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SetupError::Io)?;
    if bytes.len() as u64 > MAX {
        return Err(SetupError::TooLarge);
    }
    Ok(Some(bytes))
}
fn read_manifest(path: &Path, home: &Path) -> Result<Option<Manifest>, SetupError> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(None);
    };
    let m: Manifest = serde_json::from_slice(&bytes).map_err(|_| SetupError::Conflict)?;
    if m.schema_version != 1
        || m.home != home
        || uuid::Uuid::parse_str(&m.installation_token).is_err()
        || !matches!(m.phase.as_str(), "preparing" | "complete")
        || m.directory_inode.is_none()
        || m.stage_name.as_ref().is_some_and(|stage| {
            m.phase != "preparing"
                || stage != &format!(".herdr-threads-stage-{}", m.installation_token)
        })
        || m.assets.len() != 3
        || NAMES.iter().any(|n| {
            m.assets
                .get(*n)
                .is_none_or(|a| a.bytes.len() as u64 > MAX || a.digest != fingerprint(&a.bytes))
        })
    {
        return Err(SetupError::Conflict);
    }
    let settings: serde_json::Value = serde_json::from_slice(&m.assets["bridge_config.json"].bytes)
        .map_err(|_| SetupError::Conflict)?;
    if settings["installation_token"] != m.installation_token {
        return Err(SetupError::Conflict);
    }
    Ok(Some(m))
}
fn save_manifest(path: &Path, m: &Manifest) -> Result<(), SetupError> {
    let bytes = serde_json::to_vec(m).map_err(|_| SetupError::Invalid)?;
    if bytes.len() as u64 > MAX {
        return Err(SetupError::TooLarge);
    }
    write_replacement(path, &bytes, true)
}
fn current_digest(path: &Path) -> Result<Option<String>, SetupError> {
    Ok(read_optional(path)?.map(|b| fingerprint(&b)))
}
fn validate_scope(o: &ProfileObservation) -> Result<(), SetupError> {
    if !valid_path(&o.home)
        || o.home.canonicalize().map_err(|_| SetupError::Invalid)? != o.physical_home
    {
        return Err(SetupError::Conflict);
    }
    Ok(())
}
fn generation_directory(lock: &ProfileLock, m: &Manifest) -> Result<PathBuf, SetupError> {
    lock.validate()?;
    if identity(&lock.file.metadata().map_err(|_| SetupError::Io)?) != m.lock_inode {
        return Err(SetupError::Conflict);
    }
    let final_dir = lock.plugins.join("herdr-threads");
    let dir = match fs::symlink_metadata(&final_dir) {
        Ok(_) => final_dir,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && m.phase == "preparing" => lock
            .plugins
            .join(m.stage_name.as_ref().ok_or(SetupError::Conflict)?),
        _ => return Err(SetupError::Conflict),
    };
    let meta = fs::symlink_metadata(&dir).map_err(|_| SetupError::Conflict)?;
    if !meta.is_dir() || m.directory_inode != Some(identity(&meta)) {
        return Err(SetupError::Conflict);
    }
    Ok(dir)
}
fn verify_generation(lock: &ProfileLock, m: &Manifest) -> Result<(), SetupError> {
    let dir = generation_directory(lock, m)?;

    for (name, a) in &m.assets {
        let current = current_digest(&dir.join(name))?;
        let accepted = if m.phase == "complete" {
            current.as_deref() == Some(&a.digest)
        } else {
            current.as_deref() == Some(&a.digest) || current == a.prior
        };
        if !accepted {
            return Err(SetupError::Conflict);
        }
    }
    Ok(())
}
fn publish(lock: &ProfileLock, manifest: &Path, m: &mut Manifest) -> Result<(), SetupError> {
    verify_generation(lock, m)?;
    let source = generation_directory(lock, m)?;
    let dir = lock.plugins.join("herdr-threads");
    if source != dir {
        lock.validate()?;
        if fs::symlink_metadata(&dir).is_ok() {
            return Err(SetupError::Conflict);
        }
        fs::rename(&source, &dir).map_err(|_| SetupError::Io)?;
        File::open(&lock.plugins)
            .and_then(|f| f.sync_all())
            .map_err(|_| SetupError::Io)?;
    }
    regular_dir(&dir, false)?;
    let dir_inode = identity(&fs::symlink_metadata(&dir).map_err(|_| SetupError::Io)?);
    for (name, a) in &m.assets {
        lock.validate()?;
        if identity(&fs::symlink_metadata(&dir).map_err(|_| SetupError::Conflict)?) != dir_inode {
            return Err(SetupError::Conflict);
        }
        let current = current_digest(&dir.join(name))?;
        if current.as_deref() != Some(&a.digest) && current != a.prior {
            return Err(SetupError::Conflict);
        }
        write_replacement(&dir.join(name), &a.bytes, true)?;
    }
    lock.validate()?;
    m.phase = "complete".into();
    m.stage_name = None;
    save_manifest(manifest, m)
}

#[derive(Debug, PartialEq, Eq)]
pub enum InspectionError {
    Observation(super::runtime::ProbeFailure),
    Assets(SetupError),
}
/// Real producer-to-asset consumer seam, deliberately outside admission/registry.
pub fn setup_selected_profile(
    inspection: &super::runtime::ProfileInspection<'_>,
    state: &Path,
    rust: &Path,
    host: &Path,
) -> Result<AssetStatus, InspectionError> {
    let observed = inspection.observe().map_err(InspectionError::Observation)?;
    setup(&observed, state, rust, host).map_err(InspectionError::Assets)?;
    status(&observed, state).map_err(InspectionError::Assets)
}
pub fn status_selected_profile(
    inspection: &super::runtime::ProfileInspection<'_>,
    state: &Path,
) -> Result<AssetStatus, InspectionError> {
    let observed = inspection.observe().map_err(InspectionError::Observation)?;
    status(&observed, state).map_err(InspectionError::Assets)
}

/// Place the completed bridge/config under the observed home; never edit native YAML.
pub fn setup(
    o: &ProfileObservation,
    state: &Path,
    rust: &Path,
    host: &Path,
) -> Result<(), SetupError> {
    validate_scope(o)?;
    if !valid_path(rust) || !valid_path(host) {
        return Err(SetupError::Invalid);
    }
    let lock = ProfileLock::acquire(&o.physical_home, true, true)?;
    validate_scope(o)?;
    let manifest = manifest_path(state, &o.physical_home, true)?;
    let dir = lock.plugins.join("herdr-threads");
    let mut old = read_manifest(&manifest, &o.physical_home)?;
    if let Some(m) = old.as_mut() {
        if m.phase == "preparing" {
            publish(&lock, &manifest, m)?;
        }
        verify_generation(&lock, m)?;
    } else if fs::symlink_metadata(&dir).is_ok() {
        return Err(SetupError::Conflict);
    }
    let token = uuid::Uuid::new_v4().to_string();
    let config=serde_json::to_vec(&serde_json::json!({"schema_version":1,"rust_executable":rust,"state_root":state,"host_endpoint":host,"bridge_schema_version":1,"installation_token":token})).map_err(|_|SetupError::Invalid)?;
    let contents = [
        include_bytes!("../../../integrations/hermes/__init__.py").to_vec(),
        config,
        include_bytes!("../../../integrations/hermes/plugin.yaml").to_vec(),
    ];
    let assets = NAMES
        .into_iter()
        .zip(contents)
        .map(|(name, bytes)| {
            let prior = old.as_ref().map(|m| m.assets[name].digest.clone());
            (
                name.into(),
                Asset {
                    digest: fingerprint(&bytes),
                    prior,
                    bytes,
                },
            )
        })
        .collect();
    let stage_name = if old.is_none() {
        Some(format!(".herdr-threads-stage-{token}"))
    } else {
        None
    };
    let directory_inode = if let Some(stage) = &stage_name {
        let path = lock.plugins.join(stage);
        crate::daemon::paths::ensure_private_dir(&path).map_err(|_| SetupError::Invalid)?;
        Some(identity(
            &fs::symlink_metadata(&path).map_err(|_| SetupError::Io)?,
        ))
    } else {
        old.as_ref().and_then(|m| m.directory_inode)
    };
    let mut new = Manifest {
        schema_version: 1,
        home: o.physical_home.clone(),
        installation_token: token,
        phase: "preparing".into(),
        lock_inode: identity(&lock.file.metadata().map_err(|_| SetupError::Io)?),
        directory_inode,
        stage_name,
        assets,
    };
    lock.validate()?;
    validate_scope(o)?;
    if let Err(error) = save_manifest(&manifest, &new) {
        if let Some(stage) = &new.stage_name {
            let _ = fs::remove_dir(lock.plugins.join(stage));
        }
        return Err(error);
    }
    publish(&lock, &manifest, &mut new)?;
    validate_scope(o)?;
    Ok(())
}

pub fn status(o: &ProfileObservation, state: &Path) -> Result<AssetStatus, SetupError> {
    validate_scope(o)?;
    let mut result = AssetStatus {
        installed: false,
        repairable: false,
        configured_enabled: o.configured_enabled(),
        manual_argv: guidance(&o.profile, "enable"),
    };
    if matches!(fs::symlink_metadata(o.physical_home.join("plugins")),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
    {
        return Ok(result);
    }
    let lock = ProfileLock::acquire(&o.physical_home, false, false)?;
    validate_scope(o)?;
    let manifest = manifest_path(state, &o.physical_home, false)?;
    if let Some(m) = read_manifest(&manifest, &o.physical_home)? {
        verify_generation(&lock, &m)?;
        result.installed = m.phase == "complete";
        result.repairable = m.phase == "preparing";
    } else if lock.plugins.join("herdr-threads").exists() {
        return Err(SetupError::Conflict);
    }
    Ok(result)
}

/// Removal uses persisted ownership only; no native executable/version needed.
pub fn unsetup(home: &Path, profile: &str, state: &Path) -> Result<Removal, SetupError> {
    if !valid_path(home)
        || !valid_path(state)
        || profile.is_empty()
        || profile.len() > 256
        || profile.chars().any(char::is_control)
    {
        return Err(SetupError::Invalid);
    }
    let mut result = Removal {
        residue: vec![],
        manual_argv: guidance(profile, "disable"),
    };
    if matches!(fs::symlink_metadata(home.join("plugins")),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
    {
        return Ok(result);
    }
    let lock = ProfileLock::acquire(home, true, false)?;
    let manifest = manifest_path(state, home, false)?;
    let Some(m) = read_manifest(&manifest, home)? else {
        if lock.plugins.join("herdr-threads").exists() {
            return Err(SetupError::Conflict);
        }
        return Ok(result);
    };
    let final_dir = lock.plugins.join("herdr-threads");
    let dir = if matches!(fs::symlink_metadata(&final_dir),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
        && m.stage_name.is_none()
    {
        // An interrupted exact removal may have already removed the empty directory.
        lock.validate()?;
        if identity(&lock.file.metadata().map_err(|_| SetupError::Io)?) != m.lock_inode {
            return Err(SetupError::Conflict);
        }
        fs::remove_file(manifest).map_err(|_| SetupError::Io)?;
        return Ok(result);
    } else {
        generation_directory(&lock, &m)?
    };
    // Inspect one locked generation. Never delete a successor using stale state.
    for (name, a) in &m.assets {
        let current = current_digest(&dir.join(name))?;
        if current.is_some()
            && current.as_deref() != Some(&a.digest)
            && !(m.phase == "preparing" && current == a.prior)
        {
            result.residue.push(name.clone());
        }
    }
    if !result.residue.is_empty() {
        return Ok(result);
    }
    lock.validate()?;
    for name in NAMES {
        lock.validate()?;
        let path = dir.join(name);
        if let Some(bytes) = read_optional(&path)? {
            let a = &m.assets[name];
            let digest = fingerprint(&bytes);
            if digest != a.digest && !(m.phase == "preparing" && a.prior.as_ref() == Some(&digest))
            {
                return Err(SetupError::Conflict);
            }
            let meta = fs::symlink_metadata(&dir).map_err(|_| SetupError::Conflict)?;
            if !meta.is_dir() || m.directory_inode != Some(identity(&meta)) {
                return Err(SetupError::Conflict);
            }
            fs::remove_file(path).map_err(|_| SetupError::Io)?;
        }
    }
    lock.validate()?;
    if dir.exists() {
        match fs::remove_dir(&dir) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                result.residue.push("herdr-threads/".into())
            }
            Err(_) => return Err(SetupError::Io),
        }
    }
    if result.residue.is_empty() {
        fs::remove_file(manifest).map_err(|_| SetupError::Io)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::runtime::{ConfigQuality, ProfileObservation};
    use super::*;
    use crate::harness::runtime::RuntimeDescriptor;
    use std::{fs, path::PathBuf};
    struct Fixture {
        root: PathBuf,
        home: PathBuf,
        state: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("asset Ω {}", uuid::Uuid::new_v4()));
            let home = root.join("selected home");
            let state = root.join("state one");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir(&state).unwrap();
            Self { root, home, state }
        }
        fn observation(&self) -> ProfileObservation {
            ProfileObservation {
                identity: RuntimeDescriptor {
                    release_version: None,
                    source: "git".into(),
                    base_version: Some("0.21.5".into()),
                    derived_version: Some("0.21.5+1.g1234567".into()),
                    commit: Some("1234567890abcdef1234567890abcdef12345678".into()),
                    dirty: Some(false),
                    distance: Some(1),
                },
                profile: "default".into(),
                home: self.home.clone(),
                physical_home: self.home.canonicalize().unwrap(),
                config_quality: ConfigQuality::Successful,
                enabled: Some(vec!["herdr-threads".into()]),
                disabled: Some(vec!["herdr-threads".into()]),
            }
        }
        fn install(&self) -> Result<(), SetupError> {
            setup(
                &self.observation(),
                &self.state,
                &self.root.join("rust executable"),
                &self.root.join("host.socket"),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn stages_completed_bridge_and_six_settings_in_observed_home() {
        // Missing producer-consumer placement, stale skeleton bytes, or wrong
        // config names must break the actual asset consumer.
        let f = Fixture::new();
        assert_eq!(f.install(), Ok(()));
        let dir = f.home.join("plugins/herdr-threads");
        assert_eq!(
            fs::read(dir.join("__init__.py")).unwrap(),
            include_bytes!("../../../integrations/hermes/__init__.py")
        );
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("bridge_config.json")).unwrap()).unwrap();
        assert_eq!(config.as_object().unwrap().len(), 6);
        assert_eq!(
            config["rust_executable"],
            f.root.join("rust executable").to_str().unwrap()
        );
        assert_eq!(config["bridge_schema_version"], 1);
        assert!(!f.home.join("config.yaml").exists());
    }
    #[test]
    fn hermes_owned_assets_share_physical_profile_lock_and_remove_only_locked_generation() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        f.install().unwrap();
        let lock_path = f.home.join("plugins").join(LOCK);
        let original = identity(&fs::metadata(&lock_path).unwrap());
        let lock = ProfileLock::acquire(&f.home, true, false).unwrap();
        let alias = f.root.join("lexical alias");
        symlink(&f.home, &alias).unwrap();
        let mut o = f.observation();
        o.home = alias;
        let second = f.root.join("state two");
        fs::create_dir(&second).unwrap();
        assert_eq!(
            setup(&o, &second, &f.root.join("rust"), &f.root.join("host")),
            Err(SetupError::Conflict)
        );
        assert_eq!(status(&o, &f.state), Err(SetupError::Conflict));
        assert_eq!(
            unsetup(&f.home, "default", &f.state),
            Err(SetupError::Conflict)
        );
        drop(lock);
        assert_eq!(
            setup(&o, &second, &f.root.join("rust"), &f.root.join("host")),
            Err(SetupError::Conflict)
        );
        let path = manifest_path(&f.state, &f.home, false).unwrap();
        let stale = fs::read(&path).unwrap();
        f.install().unwrap();
        let successor = fs::read(f.home.join("plugins/herdr-threads/bridge_config.json")).unwrap();
        fs::write(&path, &stale).unwrap();
        assert!(
            !unsetup(&f.home, "default", &f.state)
                .unwrap()
                .residue
                .is_empty()
        );
        assert_eq!(
            fs::read(f.home.join("plugins/herdr-threads/bridge_config.json")).unwrap(),
            successor
        );
        // Put the correct manifest back by restoring its settings generation
        // in this test-only fixture, then exact removal retains the lock inode.
        fs::write(
            f.home.join("plugins/herdr-threads/bridge_config.json"),
            read_manifest(&path, &f.home).unwrap().unwrap().assets["bridge_config.json"]
                .bytes
                .clone(),
        )
        .unwrap();
        assert!(
            unsetup(&f.home, "default", &f.state)
                .unwrap()
                .residue
                .is_empty()
        );
        assert_eq!(identity(&fs::metadata(lock_path).unwrap()), original);
    }

    #[test]
    fn default_named_isolation_interruption_and_config_selection_are_independent() {
        let f = Fixture::new();
        f.install().unwrap();
        assert_eq!(
            status(&f.observation(), &f.state)
                .unwrap()
                .configured_enabled,
            Some(false)
        );
        let mut enabled = f.observation();
        enabled.disabled = Some(vec![]);
        assert_eq!(
            status(&enabled, &f.state).unwrap().configured_enabled,
            Some(true)
        );
        enabled.config_quality = ConfigQuality::FailedConfigRead;
        enabled.enabled = None;
        enabled.disabled = None;
        assert_eq!(status(&enabled, &f.state).unwrap().configured_enabled, None);
        let named = f.home.join("profiles/work");
        fs::create_dir_all(&named).unwrap();
        let mut o = f.observation();
        o.home = named.clone();
        o.physical_home = named.clone();
        o.profile = "work".into();
        setup(&o, &f.state, &f.root.join("rust"), &f.root.join("socket")).unwrap();
        let path = manifest_path(&f.state, &named, false).unwrap();
        let mut m = read_manifest(&path, &named).unwrap().unwrap();
        m.phase = "preparing".into();
        for a in m.assets.values_mut() {
            a.prior = None;
        }
        save_manifest(&path, &m).unwrap();
        fs::remove_file(named.join("plugins/herdr-threads/plugin.yaml")).unwrap();
        let partial = status(&o, &f.state).unwrap();
        assert!(!partial.installed);
        assert!(partial.repairable);
        setup(&o, &f.state, &f.root.join("rust"), &f.root.join("socket")).unwrap();
        assert!(status(&o, &f.state).unwrap().installed);
        assert!(
            unsetup(&named, "work", &f.state)
                .unwrap()
                .residue
                .is_empty()
        );
        assert!(status(&f.observation(), &f.state).unwrap().installed);
    }

    #[test]
    fn foreign_modified_and_missing_unsetup_never_claim_or_create_assets() {
        let f = Fixture::new();
        assert!(
            unsetup(&f.home, "default", &f.state)
                .unwrap()
                .residue
                .is_empty()
        );
        assert!(!f.home.join("plugins").exists());
        assert!(!f.state.join("setup").exists());
        f.install().unwrap();
        let dir = f.home.join("plugins/herdr-threads");
        fs::write(dir.join("__init__.py"), b"user changes").unwrap();
        assert_eq!(f.install(), Err(SetupError::Conflict));
        assert_eq!(
            unsetup(&f.home, "default", &f.state).unwrap().residue,
            vec!["__init__.py"]
        );
        assert_eq!(fs::read(dir.join("__init__.py")).unwrap(), b"user changes");
        assert!(dir.join("bridge_config.json").exists());
    }

    #[test]
    fn symlink_asset_directory_and_replaced_lock_inode_refuse_generation() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        f.install().unwrap();
        let dir = f.home.join("plugins/herdr-threads");
        let other = f.root.join("foreign identical bytes");
        fs::rename(&dir, &other).unwrap();
        symlink(&other, &dir).unwrap();
        assert_eq!(
            status(&f.observation(), &f.state),
            Err(SetupError::Conflict)
        );
        assert_eq!(
            unsetup(&f.home, "default", &f.state),
            Err(SetupError::Conflict)
        );
        assert!(other.join("bridge_config.json").exists());
        fs::remove_file(&dir).unwrap();
        fs::rename(&other, &dir).unwrap();
    }

    #[test]
    fn replaced_lock_inode_refuses_existing_manifest_generation() {
        let f = Fixture::new();
        f.install().unwrap();
        let lock = f.home.join("plugins").join(LOCK);
        fs::rename(&lock, f.root.join("old lock")).unwrap();
        let replacement = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&lock)
            .unwrap();
        drop(replacement);
        assert_eq!(f.install(), Err(SetupError::Conflict));
        assert_eq!(
            unsetup(&f.home, "default", &f.state),
            Err(SetupError::Conflict)
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn lock_child() {
        let Some(home) = std::env::var_os("HT_ASSET_LOCK_HOME") else {
            return;
        };
        let home = PathBuf::from(home);
        let _held = ProfileLock::acquire(&home, true, false).unwrap();
        fs::write(home.join("lock-ready"), b"ready").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
    #[test]
    #[cfg(feature = "test-support")]
    fn process_death_releases_physical_lock_without_replacing_inode() {
        use crate::test_support::spawn::SpawnOwned;
        let f = Fixture::new();
        f.install().unwrap();
        let inode = identity(&fs::metadata(f.home.join("plugins").join(LOCK)).unwrap());
        let mut command = crate::test_support::spawn::command(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "harness::hermes::assets::tests::lock_child",
                "--nocapture",
            ])
            .env("HT_ASSET_LOCK_HOME", &f.home)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = command.spawn_owned().unwrap();
        let start = std::time::Instant::now();
        while !f.home.join("lock-ready").exists()
            && start.elapsed() < std::time::Duration::from_secs(2)
        {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(f.home.join("lock-ready").exists());
        assert_eq!(f.install(), Err(SetupError::Conflict));
        child.stop();
        assert!(f.install().is_ok());
        assert_eq!(
            identity(&fs::metadata(f.home.join("plugins").join(LOCK)).unwrap()),
            inode
        );
    }
    #[test]
    #[cfg(feature = "test-support")]
    fn synthetic_native_entry_reaches_asset_setup_status_and_actual_bridge_settings_reader() {
        use crate::test_support::spawn::SpawnOwned;
        use std::os::unix::fs::PermissionsExt;
        // Python is only a synthetic-fixture generator here. The actual helper
        // child uses the interpreter returned by that fixture's machine argv.
        let f = Fixture::new();
        let integration = Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations/hermes");
        let script = "import sys,json;from pathlib import Path;sys.path.insert(0,sys.argv[1]);from test_runtime_helper import RuntimeHelperTests;t=RuntimeHelperTests();t.setUp();t.tmp._finalizer.detach();print(json.dumps({'python':str(Path(sys.executable).resolve()),'native':str(t.native),'selected':str(t.selected),'home':str(t.home)}))";
        let mut command = crate::test_support::spawn::command("python3");
        command
            .args(["-I", "-B", "-c", script])
            .arg(&integration)
            .env("TMPDIR", &f.root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let output = command.spawn_owned().unwrap().wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let fixture: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let python = PathBuf::from(fixture["python"].as_str().unwrap());
        let native = PathBuf::from(fixture["native"].as_str().unwrap());
        let home = PathBuf::from(fixture["home"].as_str().unwrap());
        let helper = integration.join("runtime_helper.py");
        let opaque = format!(
            "import sys,runpy;sys.path.insert(0,{:?});import hermes_bootstrap;runpy.run_module('trace',run_name='__main__',alter_sys=True)",
            native.to_str().unwrap()
        );
        let argv = serde_json::json!([
            python,
            "-I",
            "-c",
            opaque,
            "--count",
            "--no-report",
            helper,
            "--profile",
            "default"
        ]);
        let launcher = f.root.join("synthetic launcher");
        fs::write(
            &launcher,
            format!("#!/bin/sh\n/bin/cat <<'JSON'\n{argv}\nJSON\n"),
        )
        .unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
        let mut environment = crate::harness::adapter::SetupEnvironment {
            home: Some(f.root.as_os_str().to_owned()),
            cwd: f.root.clone(),
            ..Default::default()
        };
        environment
            .declared
            .insert("HERMES_HOME".into(), home.as_os_str().to_owned());
        environment
            .declared
            .insert("FIXTURE_HOME".into(), home.as_os_str().to_owned());
        environment.declared.insert(
            "FIXTURE_DEP".into(),
            fixture["selected"].as_str().unwrap().into(),
        );
        let scope = super::super::runtime::MetadataScope {
            interpreter: python.clone(),
            source_root: native,
            profile: "default".into(),
            home: home.clone(),
        };
        let budget = crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(5000),
            cancellation: Default::default(),
        };
        let inspection = super::super::runtime::ProfileInspection {
            launcher: &launcher,
            helper: &helper,
            scope: &scope,
            environment: &environment,
            budget: &budget,
        };
        let result = setup_selected_profile(
            &inspection,
            &f.state,
            &f.root.join("rust executable"),
            &f.root.join("host.socket"),
        )
        .unwrap();
        assert!(result.installed);
        assert_eq!(result.configured_enabled, Some(true));
        assert!(
            status_selected_profile(&inspection, &f.state)
                .unwrap()
                .installed
        );
        // Exercise completed Task15's settings reader on the actual Rust-staged
        // bridge/config. Importing this owned module registers no native hooks.
        let dir = home.join("plugins/herdr-threads");
        let script = "import sys,json,importlib.util;from pathlib import Path;p=Path(sys.argv[1]);s=importlib.util.spec_from_file_location('synthetic_owned_bridge',p/'__init__.py');m=importlib.util.module_from_spec(s);s.loader.exec_module(m);print(json.dumps(m.read_settings(p)))";
        let mut command = crate::test_support::spawn::command(&python);
        command
            .args(["-I", "-B", "-c", script])
            .arg(&dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let output = command.spawn_owned().unwrap().wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let settings: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(settings["state_root"], f.state.to_str().unwrap());
        assert_eq!(
            settings["rust_executable"],
            f.root.join("rust executable").to_str().unwrap()
        );
        assert!(
            unsetup(&home, "default", &f.state)
                .unwrap()
                .residue
                .is_empty()
        );
        assert!(super::super::runtime::qualify_installed().is_err());
    }
    #[test]
    fn lock_symlink_home_replacement_and_oversized_assets_refuse() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        f.install().unwrap();
        let lock_path = f.home.join("plugins").join(LOCK);
        let original = f.root.join("original lock");
        fs::rename(&lock_path, &original).unwrap();
        symlink(&original, &lock_path).unwrap();
        assert_eq!(f.install(), Err(SetupError::Conflict));
        assert_eq!(
            status(&f.observation(), &f.state),
            Err(SetupError::Conflict)
        );
        fs::remove_file(&lock_path).unwrap();
        fs::rename(&original, &lock_path).unwrap();
        fs::write(
            f.home.join("plugins/herdr-threads/__init__.py"),
            vec![b'x'; 1_048_577],
        )
        .unwrap();
        assert_eq!(
            status(&f.observation(), &f.state),
            Err(SetupError::Conflict)
        );
        assert_eq!(
            unsetup(&f.home, "default", &f.state),
            Err(SetupError::Conflict)
        );
        let held = ProfileLock::acquire(&f.home, true, false).unwrap();
        let old = f.root.join("old home");
        fs::rename(&f.home, &old).unwrap();
        fs::create_dir(&f.home).unwrap();
        assert_eq!(held.validate(), Err(SetupError::Conflict));
    }
    #[test]
    fn interrupted_directory_publication_recovers_recorded_staging_inode() {
        let f = Fixture::new();
        f.install().unwrap();
        let manifest = manifest_path(&f.state, &f.home, false).unwrap();
        let mut m: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        m["phase"] = serde_json::json!("preparing");
        let token = m["installation_token"].as_str().unwrap();
        let stage = format!(".herdr-threads-stage-{token}");
        m["stage_name"] = serde_json::json!(stage);
        for a in m["assets"].as_object_mut().unwrap().values_mut() {
            a["prior"] = serde_json::Value::Null;
        }
        fs::rename(
            f.home.join("plugins/herdr-threads"),
            f.home.join("plugins").join(&stage),
        )
        .unwrap();
        fs::write(&manifest, serde_json::to_vec(&m).unwrap()).unwrap();
        let interrupted = status(&f.observation(), &f.state).unwrap();
        assert!(!interrupted.installed);
        assert!(interrupted.repairable);
        f.install().unwrap();
        assert!(status(&f.observation(), &f.state).unwrap().installed);
        assert!(!f.home.join("plugins").join(stage).exists());
    }
    #[test]
    fn missing_persistent_lock_is_refused_without_creating_a_new_inode() {
        let f = Fixture::new();
        f.install().unwrap();
        let lock = f.home.join("plugins").join(LOCK);
        fs::remove_file(&lock).unwrap();
        assert_eq!(f.install(), Err(SetupError::Conflict));
        assert!(
            !lock.exists(),
            "an existing generation must not recreate its lock inode"
        );
        assert!(
            f.home
                .join("plugins/herdr-threads/bridge_config.json")
                .exists()
        );
    }
}
