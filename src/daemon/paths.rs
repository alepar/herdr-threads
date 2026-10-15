//! Runtime paths of this machine+user's node, resolved from the state directory.

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
    /// The claim host: the Herdr server this call attaches to (an attach hint,
    /// never part of the node's address). `None` for a store-only call; read it
    /// through [`RuntimeContext::require_host`] when a Herdr server is needed.
    pub host_endpoint: Option<PathBuf>,
    pub herdr_bin: Option<PathBuf>,
}

impl RuntimeContext {
    pub fn explicit(
        state_dir: PathBuf,
        host_endpoint: impl Into<Option<PathBuf>>,
        herdr_bin: Option<PathBuf>,
    ) -> io::Result<Self> {
        let host_endpoint = host_endpoint.into();
        if !state_dir.is_absolute() || host_endpoint.as_ref().is_some_and(|h| !h.is_absolute()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "state and host endpoint must be absolute",
            ));
        }
        if state_dir.as_os_str().is_empty()
            || host_endpoint
                .as_ref()
                .is_some_and(|h| h.file_name().is_none())
        {
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
        let host_endpoint = host_endpoint
            .map(|host| -> io::Result<PathBuf> {
                let parent = host.parent().unwrap().canonicalize()?;
                Ok(parent.join(host.file_name().unwrap()))
            })
            .transpose()?;
        Ok(Self {
            state_dir,
            host_endpoint,
            herdr_bin,
        })
    }

    /// Responsibility: the one place that produces the "host endpoint missing"
    /// error. Every consumer that needs a pane or a Herdr server (hook claim,
    /// `me`, `launch`, handoff, `panes`, setup) reads the claim host through
    /// this instead of the field. Does not: decide whether a call needs a host
    /// (the caller does) or attach it (the daemon attaches the claim host
    /// lazily, `DomainService::attached_host`).
    pub fn require_host(&self) -> io::Result<&Path> {
        self.host_endpoint.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "host endpoint missing: use --host-endpoint or HERDR_SOCKET_PATH",
            )
        })
    }

    /// The claim host as the wire locator every `CallerClaim` and target-bearing command carries:
    /// [`RuntimeContext::require_host`], canonical by construction, parsed without touching the
    /// filesystem. A path `HostLocator::parse` refuses is an `InvalidInput` error.
    pub fn claim_host(&self) -> io::Result<crate::protocol::ids::HostLocator> {
        let host = self.require_host()?;
        let text = host.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "host endpoint is not UTF-8")
        })?;
        crate::protocol::ids::HostLocator::parse(text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.detail))
    }

    /// Explicit values take precedence. The state directory is required; the
    /// claim host is optional (see [`RuntimeContext::require_host`]).
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
        let host =
            host_endpoint.or_else(|| std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from));
        Self::explicit(
            state,
            host,
            std::env::var_os("HERDR_BIN_PATH").map(PathBuf::from),
        )
    }
}

/// Every path of this machine+user's node, resolved from the state directory
/// alone (no Herdr host input): `<state>/node/{node.key, node.json,
/// herdr-threads.db, owner.lock, endpoint.json, daemon.sock, settings.json,
/// logs/, daemon.log}`.
///
/// Owns: the node layout under `<state>/node`, the socket-length fallback and
/// the private-directory preparation. Does not: open or create anything under
/// the legacy `instances/` tree (only [`legacy_instances_dir`] names it, so
/// diagnostics can report it) or know which Herdr servers the daemon talks
/// to. A new node file is a new field here, resolved from `node_dir`.
#[derive(Debug, Clone)]
pub struct NodePaths {
    pub state_dir: PathBuf,
    pub node_dir: PathBuf,
    pub key_path: PathBuf,
    pub node_json_path: PathBuf,
    pub database_path: PathBuf,
    pub lock_path: PathBuf,
    pub descriptor_path: PathBuf,
    pub socket_path: PathBuf,
}

/// The store database's file name inside the node directory.
pub const DATABASE_FILE: &str = "herdr-threads.db";

/// The pre-node-layout store file name, looked for only under `instances/*/`
/// so a state directory that holds user data is still preferred.
const LEGACY_DATABASE_FILE: &str = "threads.sqlite3";

/// The legacy per-host `instances/` directory of a state root. Reported by
/// diagnostics, never opened or created by the daemon.
pub fn legacy_instances_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("instances")
}

/// Whether `state_dir` holds a store: `node/herdr-threads.db`, or some legacy
/// `instances/<id>/threads.sqlite3`. A state directory that only ever held a
/// socket, a lock or setup manifests (a leftover from an uninstalled plugin)
/// does not.
pub fn holds_store(state_dir: &Path) -> bool {
    node_dir(state_dir).join(DATABASE_FILE).is_file()
        || std::fs::read_dir(legacy_instances_dir(state_dir)).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().join(LEGACY_DATABASE_FILE).is_file())
        })
}

impl NodePaths {
    pub fn resolve(state_dir: &Path) -> io::Result<Self> {
        Self::resolve_with_runtime_dir(state_dir, true)
    }

    /// Compute paths for diagnostics without creating the fallback socket
    /// directory. An existing fallback directory must still be private.
    pub fn resolve_read_only(state_dir: &Path) -> io::Result<Self> {
        Self::resolve_with_runtime_dir(state_dir, false)
    }

    fn resolve_with_runtime_dir(state_dir: &Path, create_runtime_dir: bool) -> io::Result<Self> {
        let node_dir = node_dir(state_dir);
        let socket_path = stable_socket_path(&node_dir)?;
        if !socket_path.starts_with(&node_dir) {
            verify_runtime_dir(socket_path.parent().unwrap(), create_runtime_dir)?;
        }
        Ok(Self {
            state_dir: state_dir.to_path_buf(),
            key_path: node_dir.join(crate::node::identity::KEY_FILE),
            node_json_path: node_dir.join(crate::node::json::NODE_JSON_FILE),
            database_path: node_dir.join(DATABASE_FILE),
            lock_path: node_dir.join("owner.lock"),
            descriptor_path: node_dir.join("endpoint.json"),
            node_dir,
            socket_path,
        })
    }

    /// Whether a published descriptor's endpoint is this node's socket.
    pub fn accepts_descriptor_endpoint(&self, endpoint: &Path) -> bool {
        endpoint == self.socket_path
    }

    /// The state root may be supplied by Herdr (0755 under the plugin state
    /// tree); it must be ours and not writable by others. Everything this
    /// plugin creates below it is a 0700 private directory.
    pub fn prepare_node_dir(&self) -> io::Result<()> {
        ensure_owned_state_root(&self.state_dir)?;
        ensure_private_dir(&self.node_dir)
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

/// The node directory of a state root: `<state>/node`.
pub fn node_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("node")
}

/// The daemon socket pathname of the node, computed without touching
/// the filesystem. It is the same for every daemon boot of the node
/// (state root), so a sandbox allowlist naming it (for
/// example Codex's `features.network_proxy.unix_sockets`) survives daemon
/// restarts. Boot identity is fenced by the endpoint descriptor and the
/// correlated boot ID of every response, never by the filename.
///
/// `<node dir>/daemon.sock` when that fits the Unix limit; otherwise
/// `<runtime dir>/<sha256(node dir)[..16]>.sock` in a private per-user
/// runtime directory (see [`select_runtime_socket_dir`]).
pub fn stable_socket_path(node_dir: &Path) -> io::Result<PathBuf> {
    let natural = node_dir.join("daemon.sock");
    let socket_path = if natural.as_os_str().as_bytes().len() < SOCKET_PATH_LIMIT {
        natural
    } else {
        let private = runtime_socket_dir();
        let digest = format!("{:x}", Sha256::digest(node_dir.as_os_str().as_bytes()));
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

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn scratch() -> Scratch {
        let dir = std::env::temp_dir().join(format!("ht-paths-{}", uuid::Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        Scratch(dir)
    }

    #[test]
    fn require_host_error_text() {
        let dir = scratch();
        let context = RuntimeContext::explicit(dir.0.clone(), None, None).unwrap();
        let error = context.require_host().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(
            error.to_string(),
            "host endpoint missing: use --host-endpoint or HERDR_SOCKET_PATH"
        );
    }

    #[test]
    fn explicit_without_host_canonicalizes_state_only() {
        let dir = scratch();
        let link = dir.0.join("link");
        std::os::unix::fs::symlink(&dir.0, &link).unwrap();
        let context = RuntimeContext::explicit(link.join("state"), None, None).unwrap();
        assert_eq!(
            context.state_dir,
            dir.0.canonicalize().unwrap().join("state")
        );
        assert_eq!(context.host_endpoint, None);
        // A present host keeps its canonical parent and is returned as is.
        let with =
            RuntimeContext::explicit(dir.0.clone(), Some(link.join("host.sock")), None).unwrap();
        assert_eq!(
            with.require_host().unwrap(),
            dir.0.canonicalize().unwrap().join("host.sock")
        );
    }

    /// The signature takes the state directory only, so neither a host
    /// endpoint nor `HERDR_SOCKET_PATH` (read here, in the ambient process
    /// environment, to prove it differs from anything the paths contain) can
    /// reach the layout.
    #[test]
    fn resolve_ignores_host_and_env() {
        let state = Path::new("/tmp/ht-state");
        let first = NodePaths::resolve_read_only(state).unwrap();
        let ambient = std::env::var("HERDR_SOCKET_PATH").unwrap_or_default();
        let second = NodePaths::resolve_read_only(state).unwrap();
        let third = NodePaths::resolve_read_only(state).unwrap();
        for path in [&first.node_dir, &first.socket_path] {
            assert!(
                ambient.is_empty() || !path.to_string_lossy().contains(&ambient),
                "{path:?} derives from HERDR_SOCKET_PATH"
            );
        }
        for other in [&second, &third] {
            assert_eq!(first.node_dir, other.node_dir);
            assert_eq!(first.socket_path, other.socket_path);
            assert_eq!(first.database_path, other.database_path);
        }
        for path in [
            &first.node_dir,
            &first.key_path,
            &first.node_json_path,
            &first.database_path,
            &first.lock_path,
            &first.descriptor_path,
            &first.socket_path,
        ] {
            assert!(path.starts_with("/tmp/ht-state/node"), "{path:?}");
        }
    }

    #[test]
    fn resolve_layout_fields() {
        let paths = NodePaths::resolve_read_only(Path::new("/tmp/ht-s")).unwrap();
        let at = |name: &str| PathBuf::from("/tmp/ht-s/node").join(name);
        assert_eq!(paths.node_dir, PathBuf::from("/tmp/ht-s/node"));
        assert_eq!(paths.key_path, at("node.key"));
        assert_eq!(paths.node_json_path, at("node.json"));
        assert_eq!(paths.database_path, at("herdr-threads.db"));
        assert_eq!(paths.lock_path, at("owner.lock"));
        assert_eq!(paths.descriptor_path, at("endpoint.json"));
        assert_eq!(paths.socket_path, at("daemon.sock"));
        assert_eq!(
            legacy_instances_dir(Path::new("/tmp/ht-s")),
            PathBuf::from("/tmp/ht-s/instances")
        );
    }

    #[test]
    fn long_state_dir_uses_stable_socket_fallback() {
        let state = PathBuf::from(format!("/tmp/{}", "s".repeat(90)));
        assert!(node_dir(&state).join("daemon.sock").as_os_str().len() >= SOCKET_PATH_LIMIT);
        let paths = NodePaths::resolve_read_only(&state).unwrap();
        assert!(!paths.socket_path.starts_with(&paths.node_dir));
        assert!(paths.socket_path.as_os_str().len() < SOCKET_PATH_LIMIT);
        assert_eq!(
            paths.socket_path,
            stable_socket_path(&paths.node_dir).unwrap()
        );
        assert_eq!(
            paths.socket_path.parent().unwrap().file_name().unwrap(),
            format!("herdr-threads-{}", effective_uid()).as_str()
        );
        assert!(paths.socket_path.extension().is_some_and(|e| e == "sock"));
    }

    #[test]
    fn holds_store_node_only_legacy_only_and_empty() {
        let empty = scratch();
        assert!(!holds_store(&empty.0));

        let node_only = scratch();
        fs::create_dir_all(node_only.0.join("node")).unwrap();
        assert!(!holds_store(&node_only.0), "a node dir without a database");
        fs::write(node_only.0.join("node").join(DATABASE_FILE), b"").unwrap();
        assert!(holds_store(&node_only.0));

        let legacy_only = scratch();
        let instance = legacy_instances_dir(&legacy_only.0).join("abc");
        fs::create_dir_all(&instance).unwrap();
        assert!(!holds_store(&legacy_only.0), "an instance dir without data");
        fs::write(instance.join("threads.sqlite3"), b"").unwrap();
        assert!(holds_store(&legacy_only.0));
        // The new database name under instances/ is not a legacy store.
        let wrong = scratch();
        let instance = legacy_instances_dir(&wrong.0).join("abc");
        fs::create_dir_all(&instance).unwrap();
        fs::write(instance.join(DATABASE_FILE), b"").unwrap();
        assert!(!holds_store(&wrong.0));
    }

    #[test]
    fn accepts_descriptor_endpoint_is_exact_socket_path() {
        let paths = NodePaths::resolve_read_only(Path::new("/tmp/ht-s")).unwrap();
        assert!(paths.accepts_descriptor_endpoint(&paths.socket_path));
        for other in [
            "/tmp/ht-s/node/other.sock",
            "/tmp/herdr-threads-1000/0123456789abcdef.sock",
            "/tmp/ht-s/node/daemon.sock.bak",
        ] {
            assert!(
                !paths.accepts_descriptor_endpoint(Path::new(other)),
                "{other}"
            );
        }
    }

    #[test]
    fn prepare_node_dir_creates_only_node_dir() {
        let state = scratch();
        let paths = NodePaths::resolve_read_only(&state.0).unwrap();
        paths.prepare_node_dir().unwrap();
        check_private_dir(&paths.node_dir).unwrap();
        assert!(!legacy_instances_dir(&state.0).exists());
    }

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
