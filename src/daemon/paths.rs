//! Scoped runtime paths for one local Herdr instance.

use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Socket pathnames stay below this many bytes: `sun_path` holds 104 bytes
/// on macOS and 108 on Linux, including the terminating NUL.
pub const SOCKET_PATH_LIMIT: usize = 100;

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    pub state_dir: PathBuf,
    pub host_endpoint: PathBuf,
    pub herdr_bin: Option<PathBuf>,
}

impl RuntimeContext {
    pub fn explicit(
        state_dir: PathBuf,
        host_endpoint: PathBuf,
        herdr_bin: Option<PathBuf>,
    ) -> io::Result<Self> {
        if !state_dir.is_absolute() || !host_endpoint.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "state and host endpoint must be absolute",
            ));
        }
        if state_dir.as_os_str().is_empty() || host_endpoint.file_name().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing state or host endpoint",
            ));
        }
        if herdr_bin.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Herdr binary path must be absolute before detach",
            ));
        }
        // Resolve the selected root before deriving socket/descriptor identities.
        // Missing state directories keep their suffix under the nearest existing
        // canonical ancestor, so first startup and subsequent clients agree too.
        let mut ancestor = state_dir.as_path();
        let mut suffix = Vec::new();
        let state_dir = loop {
            match ancestor.canonicalize() {
                Ok(mut canonical) => {
                    for component in suffix.iter().rev() {
                        canonical.push(component);
                    }
                    break canonical;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let Some(name) = ancestor.file_name() else {
                        return Err(error);
                    };
                    suffix.push(name);
                    ancestor = ancestor.parent().ok_or(error)?;
                }
                Err(error) => return Err(error),
            }
        };
        let parent = host_endpoint.parent().unwrap().canonicalize()?;
        let host_endpoint = parent.join(host_endpoint.file_name().unwrap());
        Ok(Self {
            state_dir,
            host_endpoint,
            herdr_bin,
        })
    }

    /// Explicit values take precedence. Outside a plugin action both paths are required.
    pub fn from_environment(
        state_dir: Option<PathBuf>,
        host_endpoint: Option<PathBuf>,
    ) -> io::Result<Self> {
        let state = state_dir
            .or_else(|| std::env::var_os("HERDR_PLUGIN_STATE_DIR").map(PathBuf::from))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "state directory missing: use --state-dir or HERDR_PLUGIN_STATE_DIR",
                )
            })?;
        let host = host_endpoint
            .or_else(|| std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "host endpoint missing: use --host-endpoint or HERDR_SOCKET_PATH",
                )
            })?;
        Self::explicit(
            state,
            host,
            std::env::var_os("HERDR_BIN_PATH").map(PathBuf::from),
        )
    }
}

#[derive(Debug, Clone)]
pub struct InstancePaths {
    pub instance_dir: PathBuf,
    pub socket_path: PathBuf,
    pub lock_path: PathBuf,
    pub descriptor_path: PathBuf,
    pub locator_path: PathBuf,
    pub namespace_path: PathBuf,
    pub database_path: PathBuf,
    pub locator: String,
}

/// The store database's file name inside an instance directory.
pub const DATABASE_FILE: &str = "threads.sqlite3";

/// Whether `state_dir` holds a store: some `instances/<id>/` directory contains the store
/// database. A state directory that only ever held a socket, a lock or setup manifests (a
/// leftover from an uninstalled plugin) does not.
pub fn holds_store(state_dir: &Path) -> bool {
    std::fs::read_dir(state_dir.join("instances")).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.path().join(DATABASE_FILE).is_file())
    })
}

impl InstancePaths {
    pub fn resolve(context: &RuntimeContext) -> io::Result<Self> {
        Self::resolve_with_runtime_dir(context, true)
    }

    /// Compute paths for diagnostics without creating the fallback socket
    /// directory. An existing fallback directory must still be private.
    pub fn resolve_read_only(context: &RuntimeContext) -> io::Result<Self> {
        Self::resolve_with_runtime_dir(context, false)
    }

    fn resolve_with_runtime_dir(
        context: &RuntimeContext,
        create_runtime_dir: bool,
    ) -> io::Result<Self> {
        let locator = context.host_endpoint.to_string_lossy().into_owned();
        let instance_dir = instance_dir(context);
        let socket_path = stable_socket_path(&instance_dir)?;
        if !socket_path.starts_with(&instance_dir) {
            verify_runtime_dir(socket_path.parent().unwrap(), create_runtime_dir)?;
        }
        Ok(Self {
            lock_path: instance_dir.join("owner.lock"),
            descriptor_path: instance_dir.join("endpoint.json"),
            locator_path: instance_dir.join("locator"),
            namespace_path: instance_dir.join("namespace"),
            database_path: instance_dir.join(DATABASE_FILE),
            instance_dir,
            socket_path,
            locator,
        })
    }

    /// The per-boot pathname builds before ht-910 bound
    /// (`<base stem>-<boot_id>.sock`, with the base chosen by the old length
    /// rule). A descriptor published by such a daemon is still read, so a
    /// running older daemon stays reachable and stoppable and a crashed one's
    /// socket is cleaned up by the next owner; new owners bind only
    /// [`Self::socket_path`]. Computed only; `None` if it would not fit.
    pub fn legacy_boot_socket_path(&self, boot_id: uuid::Uuid) -> Option<PathBuf> {
        let natural_boot = self
            .instance_dir
            .join(format!("daemon-{}.sock", uuid::Uuid::nil()));
        let base = if natural_boot.as_os_str().as_bytes().len() < SOCKET_PATH_LIMIT {
            self.instance_dir.join("daemon.sock")
        } else {
            let fallback = stable_socket_path(&self.instance_dir).ok()?;
            if fallback.starts_with(&self.instance_dir) {
                // The new rule keeps a natural path the old rule sent to the
                // private runtime directory: rebuild the old fallback name.
                let digest = format!(
                    "{:x}",
                    Sha256::digest(self.instance_dir.as_os_str().as_bytes())
                );
                runtime_socket_dir().join(format!("{}.sock", &digest[..16]))
            } else {
                fallback
            }
        };
        let stem = base.file_stem()?.to_os_string();
        let mut name = stem;
        name.push(format!("-{boot_id}.sock"));
        let path = base.with_file_name(name);
        (path.as_os_str().as_bytes().len() < SOCKET_PATH_LIMIT).then_some(path)
    }

    /// Whether a published descriptor's endpoint belongs to this instance:
    /// the stable pathname, the pre-ht-910 per-boot pathname for `boot_id`,
    /// or the runtime socket relocated to another runtime directory (see
    /// [`relocated_endpoint_matches`]).
    pub fn accepts_descriptor_endpoint(&self, endpoint: &Path, boot_id: uuid::Uuid) -> bool {
        endpoint == self.socket_path
            || Some(endpoint) == self.legacy_boot_socket_path(boot_id).as_deref()
            || relocated_endpoint_matches(&self.instance_dir, &self.socket_path, endpoint, boot_id)
    }

    /// The state root may be supplied by Herdr (0755 under the plugin state
    /// tree); it must be ours and not writable by others. Everything this
    /// plugin creates below it is a 0700 private directory.
    pub fn prepare_instance_dir(&self) -> io::Result<()> {
        let instances = self.instance_dir.parent().unwrap();
        let state = instances.parent().unwrap();
        ensure_owned_state_root(state)?;
        ensure_private_dir(instances)?;
        ensure_private_dir(&self.instance_dir)
    }
}

fn verify_runtime_dir(path: &Path, create: bool) -> io::Result<()> {
    if create {
        ensure_private_dir(path)
    } else {
        match check_private_dir(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// The private per-user socket directory used when an instance path is too
/// long; see [`select_runtime_socket_dir`] for the per-platform rule.
fn runtime_socket_dir() -> PathBuf {
    select_runtime_socket_dir(
        RuntimePlatform::current(),
        &|name| std::env::var_os(name),
        effective_uid(),
        &|path| path.is_dir(),
    )
}

/// The platform family the runtime directory rule distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimePlatform {
    MacOs,
    Other,
}

impl RuntimePlatform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

/// The runtime socket directory, a function of the platform and uid only so
/// it is stable for the life of a daemon (ht-3yi):
///
/// - macOS: `/private/tmp/herdr-threads-<uid>` when `/private/tmp` is a
///   directory (always, in practice), else `/tmp/herdr-threads-<uid>`;
///   unchanged from earlier releases.
/// - elsewhere always `/tmp/herdr-threads-<uid>`: `/private/tmp` is never
///   consulted, so creating it later cannot move the endpoint. Nor is the
///   environment (`XDG_RUNTIME_DIR` in particular): logind removes
///   `/run/user/<uid>` when the user's last session ends while a
///   Herdr-hosted daemon keeps running and holding the owner lock, which
///   would strand it; it would also move the socket on upgrade and make the
///   location depend on the starter's environment (ssh, cron, su, sudo).
///
/// `env` and `is_dir` are injected so tests can prove what is (not) read.
fn select_runtime_socket_dir(
    platform: RuntimePlatform,
    _env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
    uid: u32,
    is_dir: &dyn Fn(&Path) -> bool,
) -> PathBuf {
    let per_user = format!("herdr-threads-{uid}");
    let private = Path::new("/private/tmp");
    if platform == RuntimePlatform::MacOs && is_dir(private) {
        private.join(per_user)
    } else {
        Path::new("/tmp").join(per_user)
    }
}

/// Whether a descriptor's `endpoint` names this instance's runtime socket in
/// a directory another rule chose: on Linux an older build's `/private/tmp`
/// directory (picked whenever `/private/tmp` existed) versus today's `/tmp`,
/// in either direction. The check is platform-neutral, so it relaxes
/// acceptance on macOS too, where the chosen paths are unchanged. The runtime socket's
/// file name (a digest of the instance directory, or its pre-ht-910 per-boot
/// form for `boot_id`) identifies the instance wherever the socket lives;
/// the descriptor is the owner's private record and the socket is still
/// verified (owner, mode, recorded inode) before use. A natural socket inside
/// the instance directory has no relocated form.
fn relocated_endpoint_matches(
    instance_dir: &Path,
    socket_path: &Path,
    endpoint: &Path,
    boot_id: uuid::Uuid,
) -> bool {
    if socket_path.starts_with(instance_dir)
        || !endpoint.is_absolute()
        || endpoint.as_os_str().as_bytes().len() >= SOCKET_PATH_LIMIT
        || endpoint.parent().is_none_or(|dir| dir.parent().is_none())
    {
        return false;
    }
    let (Some(name), Some(stem)) = (endpoint.file_name(), socket_path.file_stem()) else {
        return false;
    };
    let mut per_boot = stem.to_os_string();
    per_boot.push(format!("-{boot_id}.sock"));
    Some(name) == socket_path.file_name() || name == per_boot
}

/// The instance directory for a runtime context: `<state>/instances/<sha256(host endpoint)>`.
pub fn instance_dir(context: &RuntimeContext) -> PathBuf {
    let digest = format!(
        "{:x}",
        Sha256::digest(context.host_endpoint.as_os_str().as_bytes())
    );
    context.state_dir.join("instances").join(digest)
}

/// The daemon socket pathname of one instance, computed without touching
/// the filesystem. It is the same for every daemon boot of that instance
/// (state root + host endpoint), so a sandbox allowlist naming it (for
/// example Codex's `features.network_proxy.unix_sockets`) survives daemon
/// restarts. Boot identity is fenced by the endpoint descriptor and the
/// correlated boot ID of every response, never by the filename.
///
/// `<instance dir>/daemon.sock` when that fits the Unix limit; otherwise
/// `<runtime dir>/<sha256(instance dir)[..16]>.sock` in a private per-user
/// runtime directory (see [`select_runtime_socket_dir`]).
pub fn stable_socket_path(instance_dir: &Path) -> io::Result<PathBuf> {
    let natural = instance_dir.join("daemon.sock");
    let socket_path = if natural.as_os_str().as_bytes().len() < SOCKET_PATH_LIMIT {
        natural
    } else {
        let private = runtime_socket_dir();
        let digest = format!("{:x}", Sha256::digest(instance_dir.as_os_str().as_bytes()));
        private.join(format!("{}.sock", &digest[..16]))
    };
    if socket_path.as_os_str().as_bytes().len() >= SOCKET_PATH_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket pathname exceeds safe Unix limit",
        ));
    }
    Ok(socket_path)
}

/// An unsafe state root or private directory is invalid local context
/// (documented exit status 2), not an unavailable daemon: restarting cannot
/// fix it. The typed payload lets callers classify it without string matching.
#[derive(Debug)]
pub struct UnsafeLocalState(String);

impl std::fmt::Display for UnsafeLocalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UnsafeLocalState {}

fn unsafe_local_state(detail: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, UnsafeLocalState(detail))
}

/// True when `error` came from the state-root or private-directory checks.
pub fn is_unsafe_local_state(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<UnsafeLocalState>())
}

/// Create the state root 0700 when absent. An existing root must be a real
/// directory owned by the effective user with no group/other write bit, so
/// Herdr's 0755 plugin state directory is accepted while a shared or
/// foreign directory is not.
pub fn ensure_owned_state_root(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    match builder.create(path) {
        Ok(()) => (),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => (),
        Err(err) => return Err(err),
    }
    check_owned_state_root(path)
}

/// Read-only form of [`ensure_owned_state_root`] for diagnostics.
pub fn check_owned_state_root(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_dir()
        || meta.uid() != effective_uid()
        || meta.permissions().mode() & 0o022 != 0
    {
        return Err(unsafe_local_state(format!(
            "unsafe state directory (must be an owned directory without group/other write): {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn ensure_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    match builder.create(path) {
        Ok(()) => (),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => (),
        Err(err) => return Err(err),
    }
    check_private_dir(path)
}

/// Read-only form of [`ensure_private_dir`] for diagnostics: an existing
/// plugin-created directory must be a real 0700 directory owned by the
/// effective user. A missing directory is reported as `NotFound`.
pub fn check_private_dir(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_dir()
        || meta.uid() != effective_uid()
        || meta.permissions().mode() & 0o777 != 0o700
    {
        return Err(unsafe_local_state(format!(
            "unsafe private directory: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn effective_uid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the selection with a fixed environment and set of existing
    /// directories; record every environment variable read and path probed.
    fn select(
        platform: RuntimePlatform,
        xdg: Option<&str>,
        uid: u32,
        dirs: &[&str],
    ) -> (PathBuf, Vec<PathBuf>) {
        let probed = std::cell::RefCell::new(Vec::new());
        let chosen = select_runtime_socket_dir(
            platform,
            &|name| {
                probed
                    .borrow_mut()
                    .push(PathBuf::from(format!("$env/{name}")));
                (name == "XDG_RUNTIME_DIR")
                    .then_some(xdg)
                    .flatten()
                    .map(std::ffi::OsString::from)
            },
            uid,
            &|path: &Path| {
                probed.borrow_mut().push(path.to_path_buf());
                dirs.iter().any(|name| Path::new(name) == path)
            },
        );
        (chosen, probed.into_inner())
    }

    /// macOS keeps today's rule exactly: `/private/tmp` when it is a
    /// directory, `/tmp` otherwise, and `XDG_RUNTIME_DIR` is never consulted.
    #[test]
    fn macos_runtime_dir_is_unchanged() {
        let (dir, _) = select(
            RuntimePlatform::MacOs,
            Some("/run/user/501"),
            501,
            &["/private/tmp", "/run/user/501"],
        );
        assert_eq!(dir, PathBuf::from("/private/tmp/herdr-threads-501"));
        let (dir, _) = select(RuntimePlatform::MacOs, None, 501, &[]);
        assert_eq!(dir, PathBuf::from("/tmp/herdr-threads-501"));
    }

    /// Regression (ht-3yi): off macOS the presence of `/private/tmp` never
    /// changes the chosen directory, and it is not even probed. Kills: a
    /// daemon stranded when `/private/tmp` is created after it started.
    #[test]
    fn non_macos_ignores_private_tmp() {
        let without = select(RuntimePlatform::Other, None, 1000, &[]);
        let with = select(RuntimePlatform::Other, None, 1000, &["/private/tmp"]);
        assert_eq!(without.0, PathBuf::from("/tmp/herdr-threads-1000"));
        assert_eq!(with.0, without.0);
        assert!(
            !with.1.iter().any(|path| path.starts_with("/private/tmp")),
            "probed {:?}",
            with.1
        );
    }

    /// Regression (review C1/I1/I2): off macOS `XDG_RUNTIME_DIR` is ignored,
    /// even when it names a valid private directory, and is never probed.
    /// logind removes `/run/user/<uid>` when the last session ends while the
    /// daemon keeps its owner lock (a stranded daemon), it would move the
    /// socket for nearly every Linux user on upgrade, and it varies with the
    /// starter's environment. Kills: reintroducing an XDG branch.
    #[test]
    fn non_macos_ignores_xdg_runtime_dir() {
        let (dir, probed) = select(
            RuntimePlatform::Other,
            Some("/run/user/1000"),
            1000,
            &["/run/user/1000"],
        );
        assert_eq!(dir, PathBuf::from("/tmp/herdr-threads-1000"));
        assert!(
            !probed
                .iter()
                .any(|path| path.starts_with("/run/user") || path.starts_with("$env")),
            "probed {probed:?}"
        );
    }

    /// A descriptor names a socket in a runtime directory chosen by another
    /// rule (an older Linux daemon under `/private/tmp`): the same instance
    /// file name is accepted wherever it lives; nothing else is.
    #[test]
    fn relocated_runtime_endpoint_is_accepted_by_instance_file_name_only() {
        let instance = Path::new("/state/instances/abc");
        let boot = uuid::Uuid::new_v4();
        let runtime = Path::new("/tmp/herdr-threads-1000/0123456789abcdef.sock");
        let accepts = |socket: &Path, endpoint: &str, boot_id| {
            relocated_endpoint_matches(instance, socket, Path::new(endpoint), boot_id)
        };
        for endpoint in [
            "/tmp/herdr-threads-1000/0123456789abcdef.sock",
            "/private/tmp/herdr-threads-1000/0123456789abcdef.sock",
            "/elsewhere/herdr-threads-1000/0123456789abcdef.sock",
        ] {
            assert!(accepts(runtime, endpoint, boot), "{endpoint}");
        }
        let per_boot = format!("/private/tmp/herdr-threads-1000/0123456789abcdef-{boot}.sock");
        assert!(accepts(runtime, &per_boot, boot));
        assert!(!accepts(runtime, &per_boot, uuid::Uuid::new_v4()));
        for endpoint in [
            "/tmp/herdr-threads-1000/fedcba9876543210.sock",
            "tmp/herdr-threads-1000/0123456789abcdef.sock",
            "/0123456789abcdef.sock",
        ] {
            assert!(!accepts(runtime, endpoint, boot), "{endpoint}");
        }
        let too_long = format!("/{}/0123456789abcdef.sock", "x".repeat(SOCKET_PATH_LIMIT));
        assert!(!accepts(runtime, &too_long, boot));
        // A natural socket inside the instance directory is matched exactly
        // by the caller; no relocated form is accepted for it.
        let natural = instance.join("daemon.sock");
        assert!(!accepts(
            &natural,
            "/tmp/herdr-threads-1000/daemon.sock",
            boot
        ));
    }

    #[test]
    fn read_only_runtime_directory_check_creates_nothing_and_keeps_unsafe_errors() {
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let scratch =
            Scratch(std::env::temp_dir().join(format!("ht-paths-{}", uuid::Uuid::new_v4())));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&scratch.0)
            .unwrap();
        let runtime = scratch.0.join("fallback");
        verify_runtime_dir(&runtime, false).unwrap();
        assert!(!runtime.exists());

        fs::DirBuilder::new().mode(0o755).create(&runtime).unwrap();
        let error = verify_runtime_dir(&runtime, false).unwrap_err();
        assert!(is_unsafe_local_state(&error), "{error}");
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        verify_runtime_dir(&runtime, false).unwrap();
    }
}
