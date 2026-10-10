//! Owned native permission components (Claude settings rules, Codex execpolicy rules): the
//! shared manifest format, consent, and the cooperative per-config-root writer lock.
//!
//! The grant is executable-wide for the bare `herdr-threads` and `ht` spellings; only the
//! immediate `human` namespace and the self-granting commands ask. The CLI grammar refuses
//! `human` and those commands anywhere but first, so leading options cannot reach them.
pub mod claude;
pub mod claude_settings;
pub mod codex;
pub mod codex_rules;

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_PATH_BYTES: usize = 4096;
pub const PERMISSION_MANIFEST_VERSION: u32 = 1;
/// The executable spellings every grant covers.
pub const SPELLINGS: [&str; 2] = ["herdr-threads", "ht"];

fn refusal(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail)
}
fn safe_absolute(path: &Path) -> io::Result<()> {
    let Some(text) = path.to_str() else {
        return Err(refusal("non-UTF8 permission path"));
    };
    if !path.is_absolute()
        || text.len() > MAX_PATH_BYTES
        || text.chars().any(char::is_control)
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(refusal(
            "permission paths must be bounded absolute paths without controls or traversal",
        ));
    }
    Ok(())
}
fn no_symlink_ancestors(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if metadata.file_type().is_symlink() {
            return Err(refusal("symlink permission path"));
        }
    }
    Ok(())
}
fn owned(metadata: &fs::Metadata) -> bool {
    metadata.uid() == unsafe { libc::geteuid() }
}
fn same_inode(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}
fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    safe_absolute(path)?;
    no_symlink_ancestors(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || !owned(&metadata) || metadata.len() > limit {
        return Err(refusal("foreign or oversized permission file"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit || !same_inode(&metadata, &fs::symlink_metadata(path)?) {
        return Err(refusal("permission file changed during read"));
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionBackend {
    Claude,
    Codex,
}
/// Exact file/rule ownership. A pre-existing entry is never removable owned data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedPermissionResource {
    pub file: PathBuf,
    pub rule: Option<String>,
    pub fingerprint: String,
    pub pre_existing: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionComponentManifest {
    pub version: u32,
    pub backend: PermissionBackend,
    pub config_root: PathBuf,
    pub resources: Vec<OwnedPermissionResource>,
    /// Native containers the component created (for Claude `permissions`, `permissions.allow`,
    /// `permissions.ask`); removal drops each one it leaves empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingPublication>,
}
/// A native file publication in flight. The manifest already records its target; the file still
/// holds `before` (not published: the previous component stands) or already holds `after`
/// (published: finish). Any other content is a conflict for the person to settle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingPublication {
    pub before: String,
    pub after: String,
    /// The component recorded before this publication; `None` when there was none.
    pub previous: Option<Box<PermissionComponentManifest>>,
    /// The owned rules of an earlier owner record (a hook manifest's historical grant) this
    /// publication takes over; that record is retired once the file holds `after`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionConsent {
    Undecided,
    Granted,
    Declined,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedConfigIdentity {
    root: PathBuf,
    backend: PermissionBackend,
    device: u64,
    inode: u64,
}
impl VerifiedConfigIdentity {
    pub fn verify(root: &Path, backend: PermissionBackend) -> io::Result<Self> {
        safe_absolute(root)?;
        no_symlink_ancestors(root)?;
        let metadata = fs::metadata(root)?;
        if !metadata.is_dir()
            || !owned(&metadata)
            || metadata.mode() & 0o022 != 0
            || root.canonicalize()? != root
        {
            return Err(refusal(
                "config root must be a verified owned effective directory",
            ));
        }
        Ok(Self {
            root: root.into(),
            backend,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn backend(&self) -> PermissionBackend {
        self.backend
    }
    pub fn revalidate(&self) -> io::Result<()> {
        if Self::verify(&self.root, self.backend)? != *self {
            return Err(refusal("effective config identity changed"));
        }
        Ok(())
    }
    fn lock_path(&self) -> PathBuf {
        self.root.join(match self.backend {
            PermissionBackend::Claude => ".herdr-threads-claude.lock",
            PermissionBackend::Codex => ".herdr-threads-codex.lock",
        })
    }
    fn marker(&self) -> Vec<u8> {
        format!(
            "herdr-threads owned config lock v1\n{:?}\n{}:{}\n{}\n",
            self.backend,
            self.device,
            self.inode,
            self.root.display()
        )
        .into_bytes()
    }
}
/// Stable owned inode, no unlink. Advisory exclusion covers cooperating writers;
/// backend final-check/rename still cannot detect a noncooperating editor in that interval.
pub struct OwnedConfigWriteGuard {
    file: File,
    identity: VerifiedConfigIdentity,
}
/// Operation-local observations for causal initialization tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigLockInitialization {
    Created,
    ReadyToPublish,
}
impl OwnedConfigWriteGuard {
    pub fn acquire(identity: &VerifiedConfigIdentity, timeout: Duration) -> io::Result<Self> {
        Self::acquire_observed(identity, timeout, |_, _| Ok(()))
    }
    #[cfg(feature = "test-support")]
    pub fn acquire_with_observer(
        identity: &VerifiedConfigIdentity,
        timeout: Duration,
        observer: impl FnMut(ConfigLockInitialization, &Path) -> io::Result<()>,
    ) -> io::Result<Self> {
        Self::acquire_observed(identity, timeout, observer)
    }
    fn acquire_observed(
        identity: &VerifiedConfigIdentity,
        timeout: Duration,
        mut observer: impl FnMut(ConfigLockInitialization, &Path) -> io::Result<()>,
    ) -> io::Result<Self> {
        if timeout > Duration::from_secs(10) {
            return Err(refusal("config lock timeout exceeds bound"));
        }
        let deadline = Instant::now() + timeout;
        identity.revalidate()?;
        let path = identity.lock_path();
        let options = || {
            let mut o = OpenOptions::new();
            o.read(true)
                .write(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW);
            o
        };
        let file = match options().open(&path) {
            Ok(file) => {
                check_lock_metadata(&file, &path)?;
                acquire_config_lock(&file, deadline)?;
                file
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // No incomplete inode is ever exposed at the stable name. A terminated
                // creator may leave this private name; it is never adopted or repaired.
                let private = identity
                    .root
                    .join(format!(".herdr-threads-lock-init-{}", uuid::Uuid::new_v4()));
                let mut file = options().create_new(true).open(&private)?;
                let mut cleanup = UnpublishedLockName::new(&private, &file)?;
                check_lock_metadata(&file, &private)?;
                observer(ConfigLockInitialization::Created, &private)?;
                file.write_all(&identity.marker())?;
                file.sync_all()?;
                acquire_config_lock(&file, deadline)?;
                observer(ConfigLockInitialization::ReadyToPublish, &private)?;
                identity.revalidate()?;
                check_lock_metadata(&file, &private)?;
                match publish_config_lock(&private, &path) {
                    Ok(()) => {
                        cleanup.path = None;
                        File::open(&identity.root)?.sync_all()?;
                        file
                    }
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        // The winning creator published a complete, already locked inode.
                        // Discard only our unpublished inode, then use the same deadline.
                        drop(file);
                        drop(cleanup);
                        let file = options().open(&path)?;
                        check_lock_metadata(&file, &path)?;
                        acquire_config_lock(&file, deadline)?;
                        file
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        };
        let guard = Self {
            file,
            identity: identity.clone(),
        };
        guard.revalidate()?;
        Ok(guard)
    }
    pub fn identity(&self) -> &VerifiedConfigIdentity {
        &self.identity
    }
    pub fn revalidate(&self) -> io::Result<()> {
        self.identity.revalidate()?;
        check_lock_metadata(&self.file, &self.identity.lock_path())?;
        // Read from a separately opened descriptor without acquiring/releasing another lock.
        if read_bounded(&self.identity.lock_path(), MAX_PATH_BYTES as u64 + 128)?
            != self.identity.marker()
        {
            return Err(refusal("foreign or edited config lock"));
        }
        Ok(())
    }
}
fn acquire_config_lock(file: &File, deadline: Instant) -> io::Result<()> {
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(fs::TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "owned config writer busy; retry after it completes",
                    ));
                }
                std::thread::sleep(remaining.min(Duration::from_millis(2)));
            }
            Err(fs::TryLockError::Error(e)) => return Err(e),
        }
    }
}
// Drop cleanup is scoped to the exact private inode this operation created.
struct UnpublishedLockName {
    path: Option<PathBuf>,
    device: u64,
    inode: u64,
}
impl UnpublishedLockName {
    fn new(path: &Path, file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            path: Some(path.into()),
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}
impl Drop for UnpublishedLockName {
    fn drop(&mut self) {
        if let Some(path) = &self.path
            && let Ok(metadata) = fs::symlink_metadata(path)
            && metadata.is_file()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(path);
        }
    }
}
fn publish_config_lock(private: &Path, stable: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let private = CString::new(private.as_os_str().as_bytes())
            .map_err(|_| refusal("invalid private config lock path"))?;
        let stable = CString::new(stable.as_os_str().as_bytes())
            .map_err(|_| refusal("invalid stable config lock path"))?;
        // SAFETY: both C strings are live and NUL terminated. These primitives
        // atomically move one name without replacing a winner or adding a link.
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(private.as_ptr(), stable.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                private.as_ptr(),
                libc::AT_FDCWD,
                stable.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (private, stable);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic no-replace config lock publication is unavailable",
        ))
    }
}
fn check_lock_metadata(file: &File, path: &Path) -> io::Result<()> {
    let metadata = file.metadata()?;
    let named = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || !owned(&metadata)
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
        || !same_inode(&metadata, &named)
        || named.file_type().is_symlink()
    {
        return Err(refusal("foreign, replaced or unsafe config lock"));
    }
    Ok(())
}
impl Drop for OwnedConfigWriteGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("ht-permissions-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&p).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(p.canonicalize().unwrap())
        }
        fn identity(&self) -> VerifiedConfigIdentity {
            VerifiedConfigIdentity::verify(&self.0, PermissionBackend::Claude).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn owned_config_guard_refuses_foreign_symlink_edited_and_replaced_inode() {
        let f = Fixture::new();
        let id = f.identity();
        let path = id.lock_path();
        assert!(OwnedConfigWriteGuard::acquire(&id, Duration::from_secs(11)).is_err());
        assert!(!path.exists());
        for bytes in [b"".as_slice(), b"herdr-threads owned config lock v1\n"] {
            fs::write(&path, bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let inode = fs::metadata(&path).unwrap().ino();
            assert!(OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
            fs::remove_file(&path).unwrap();
        }
        fs::write(&path, b"foreign").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"foreign");
        fs::remove_file(&path).unwrap();
        symlink(f.0.join("missing"), &path).unwrap();
        assert!(OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).is_err());
        fs::remove_file(&path).unwrap();
        let guard = OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(guard.revalidate().is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&path, f.0.join("extra-link")).unwrap();
        assert!(guard.revalidate().is_err());
        fs::remove_file(f.0.join("extra-link")).unwrap();
        guard.revalidate().unwrap();
        fs::write(&path, b"edited").unwrap();
        assert!(guard.revalidate().is_err());
        drop(guard);
        assert!(OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).is_err());
        fs::remove_file(&path).unwrap();
        let guard = OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).unwrap();
        fs::rename(&path, f.0.join("old-lock")).unwrap();
        fs::write(&path, id.marker()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(guard.revalidate().is_err());
    }
    // Child test re-entry uses per-command environment only, never parent-global state.
    #[cfg(feature = "test-support")]
    #[test]
    fn guard_child() {
        let Some(root) = std::env::var_os("HT_PERMISSION_GUARD_CHILD_ROOT") else {
            return;
        };
        let id =
            VerifiedConfigIdentity::verify(Path::new(&root), PermissionBackend::Claude).unwrap();
        let barrier = PathBuf::from(std::env::var_os("HT_PERMISSION_GUARD_CHILD_BARRIER").unwrap());
        match OwnedConfigWriteGuard::acquire(&id, Duration::ZERO) {
            Ok(_guard) => {
                fs::write(barrier.join("result.tmp"), b"acquired").unwrap();
                fs::rename(barrier.join("result.tmp"), barrier.join("result")).unwrap();
                let end = Instant::now() + Duration::from_secs(5);
                while !barrier.join("release").exists() {
                    assert!(Instant::now() < end, "child release timed out");
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                fs::write(barrier.join("result.tmp"), b"busy").unwrap();
                fs::rename(barrier.join("result.tmp"), barrier.join("result")).unwrap();
            }
            Err(e) => panic!("unexpected child guard error: {e}"),
        }
    }
    #[cfg(feature = "test-support")]
    #[test]
    fn owned_config_guard_excludes_other_process() {
        use crate::test_support::spawn::{SpawnOwned, command};
        let f = Fixture::new();
        let id = f.identity();
        let barrier = f.0.join("barrier");
        fs::create_dir(&barrier).unwrap();
        let lock = OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).unwrap();
        let inode = fs::metadata(id.lock_path()).unwrap().ino();
        let child = || {
            let mut command = command(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "harness::permissions::tests::guard_child",
                    "--nocapture",
                ])
                .env("HT_PERMISSION_GUARD_CHILD_ROOT", &f.0)
                .env("HT_PERMISSION_GUARD_CHILD_BARRIER", &barrier)
                .env("HOME", f.0.join("home"))
                .env("CLAUDE_CONFIG_DIR", &f.0)
                .env("CODEX_HOME", &f.0)
                .env("TMPDIR", &f.0);
            command.spawn_owned().unwrap()
        };
        let wait_result = || {
            let end = Instant::now() + Duration::from_secs(5);
            while !barrier.join("result").exists() {
                assert!(Instant::now() < end, "child result timed out");
                std::thread::sleep(Duration::from_millis(2));
            }
            fs::read(barrier.join("result")).unwrap()
        };
        let mut blocked = child();
        assert_eq!(wait_result(), b"busy");
        assert!(blocked.wait().unwrap().success());
        drop(blocked);
        drop(lock);
        fs::remove_file(barrier.join("result")).unwrap();
        let mut holder = child();
        assert_eq!(wait_result(), b"acquired");
        assert_eq!(
            OwnedConfigWriteGuard::acquire(&id, Duration::ZERO)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        fs::write(barrier.join("release"), b"release").unwrap();
        assert!(holder.wait().unwrap().success());
        drop(holder);
        let _next = OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).unwrap();
        assert_eq!(fs::metadata(id.lock_path()).unwrap().ino(), inode);
    }
}
