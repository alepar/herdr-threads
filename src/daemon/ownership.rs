//! Exclusive owner election and endpoint publication. A PID is diagnostic only.

use crate::daemon::paths::{NodePaths, effective_uid};
use crate::node::{NodeUuid, json::read_hint};
use crate::store::connection::DurableSync;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EndpointDescriptor {
    pub software_version: String,
    pub protocol_version: u16,
    pub node_uuid: NodeUuid,
    pub boot_id: Uuid,
    pub endpoint: std::path::PathBuf,
    pub pid: u32,
    pub socket_device: u64,
    pub socket_inode: u64,
}

/// The locked owner-lease file. Dropping the last holder releases the lease
/// explicitly (`flock(LOCK_UN)`) before the descriptor closes: a process
/// spawned concurrently by another thread holds a transient copy of every
/// descriptor until its exec completes, and relying on close alone kept the
/// lock held through that copy, so an immediate re-election saw WouldBlock
/// (ht-w7n). An explicit unlock releases the shared open file description
/// whatever copies exist.
#[derive(Debug)]
struct LeaseFile(File);

impl Drop for LeaseFile {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Debug)]
pub struct OwnerLock {
    lock_file: Arc<LeaseFile>,
    paths: NodePaths,
    /// From the `node.json` hint at acquire time; `None` until the daemon has
    /// written it. Publication refuses without it.
    node_uuid: Option<NodeUuid>,
    owner_token: Uuid,
}

/// The socket listener and pathname observed immediately after this owner bound it.
/// Only `OwnerLock::bind_socket` can construct this proof for publication.
#[derive(Debug)]
pub struct OwnedListener {
    listener: UnixListener,
    path: std::path::PathBuf,
    boot_id: Uuid,
    socket_device: u64,
    socket_inode: u64,
    owner_token: Uuid,
    _lock_file: Arc<LeaseFile>,
}

/// Tokio accept handle that retains the same advisory lock lease. It exposes
/// accepted streams, never the listening socket itself.
#[derive(Debug)]
pub struct OwnedAsyncListener {
    listener: tokio::net::UnixListener,
    path: std::path::PathBuf,
    boot_id: Uuid,
    socket_device: u64,
    socket_inode: u64,
    owner_token: Uuid,
    _lock_file: Arc<LeaseFile>,
}

impl OwnedListener {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn boot_id(&self) -> Uuid {
        self.boot_id
    }

    pub fn accept(
        &self,
    ) -> io::Result<(
        std::os::unix::net::UnixStream,
        std::os::unix::net::SocketAddr,
    )> {
        self.listener.accept()
    }

    /// Convert inside a Tokio runtime, keeping the lock alive in the new handle.
    pub fn into_async(self) -> io::Result<OwnedAsyncListener> {
        let Self {
            listener,
            path,
            boot_id,
            socket_device,
            socket_inode,
            owner_token,
            _lock_file,
        } = self;
        listener.set_nonblocking(true)?;
        let listener = tokio::net::UnixListener::from_std(listener)?;
        Ok(OwnedAsyncListener {
            listener,
            path,
            boot_id,
            socket_device,
            socket_inode,
            owner_token,
            _lock_file,
        })
    }
}

impl OwnedAsyncListener {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn boot_id(&self) -> Uuid {
        self.boot_id
    }
    pub async fn accept(
        &self,
    ) -> io::Result<(tokio::net::UnixStream, tokio::net::unix::SocketAddr)> {
        self.listener.accept().await
    }
}

impl OwnerLock {
    pub fn acquire(paths: &NodePaths) -> io::Result<Self> {
        paths.prepare_node_dir()?;
        let file = open_private_file(&paths.lock_path)?;
        file.try_lock()?;
        Ok(Self {
            lock_file: Arc::new(LeaseFile(file)),
            paths: paths.clone(),
            node_uuid: hint_uuid(paths),
            owner_token: Uuid::new_v4(),
        })
    }

    /// [`Self::acquire`] for fixtures that stand in for the daemon: the node
    /// key is loaded (created when absent) and its `node.json` hint written
    /// first, as the elected daemon does, so a daemon started later on the
    /// same state directory is the same node.
    #[cfg(any(test, feature = "test-support"))]
    pub fn acquire_with_test_hint(paths: &NodePaths) -> io::Result<Self> {
        crate::test_support::seed_node_identity(paths)?;
        Self::acquire(paths)
    }

    /// The node's uuid as the `node.json` hint named it when the lease was
    /// taken. The elected daemon writes the hint (from its loaded identity)
    /// before acquiring; a lease taken without one cannot publish.
    pub fn node_uuid(&self) -> io::Result<NodeUuid> {
        self.node_uuid.ok_or_else(missing_hint)
    }
    pub fn paths(&self) -> &NodePaths {
        &self.paths
    }

    /// Bind the instance's stable socket pathname for a fresh boot while this
    /// process owns the lock. The pathname is the same for every boot (a
    /// sandbox allowlist names it once); the boot is identified by the random
    /// boot ID in the descriptor and in every response, never by the name.
    pub fn bind_socket(&self) -> io::Result<OwnedListener> {
        let boot_id = Uuid::new_v4();
        let path = self.paths.socket_path.clone();
        reclaim_dead_socket(&path)?;
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let meta = validate_socket(&path)?;
        if listener.local_addr()?.as_pathname() != Some(path.as_path()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "listener path mismatch",
            ));
        }
        Ok(OwnedListener {
            listener,
            path,
            boot_id,
            socket_device: meta.dev(),
            socket_inode: meta.ino(),
            owner_token: self.owner_token,
            _lock_file: Arc::clone(&self.lock_file),
        })
    }

    /// Publish only the actual listener bound by this elected owner.
    pub fn publish_endpoint(
        &self,
        bound: &OwnedListener,
        software_version: &str,
        protocol_version: u16,
    ) -> io::Result<EndpointDescriptor> {
        self.publish_bound_endpoint(
            &bound.path,
            bound.boot_id,
            bound.socket_device,
            bound.socket_inode,
            bound.owner_token,
            bound.listener.local_addr()?.as_pathname() == Some(bound.path.as_path()),
            software_version,
            protocol_version,
        )
    }

    /// Publish after conversion to Tokio, before admitting client requests.
    pub fn publish_async_endpoint(
        &self,
        bound: &OwnedAsyncListener,
        software_version: &str,
        protocol_version: u16,
    ) -> io::Result<EndpointDescriptor> {
        self.publish_bound_endpoint(
            &bound.path,
            bound.boot_id,
            bound.socket_device,
            bound.socket_inode,
            bound.owner_token,
            bound.listener.local_addr()?.as_pathname() == Some(bound.path.as_path()),
            software_version,
            protocol_version,
        )
    }

    // Allowed: each argument is one field of the published endpoint descriptor.
    #[allow(clippy::too_many_arguments)]
    fn publish_bound_endpoint(
        &self,
        path: &Path,
        boot_id: Uuid,
        socket_device: u64,
        socket_inode: u64,
        owner_token: Uuid,
        address_matches: bool,
        software_version: &str,
        protocol_version: u16,
    ) -> io::Result<EndpointDescriptor> {
        if software_version.is_empty() || protocol_version == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing software or protocol version",
            ));
        }
        if owner_token != self.owner_token || path != self.paths.socket_path || !address_matches {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "listener does not belong to elected owner",
            ));
        }
        let socket = validate_socket(path)?;
        if socket.dev() != socket_device || socket.ino() != socket_inode {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "socket pathname was substituted",
            ));
        }
        let descriptor = EndpointDescriptor {
            software_version: software_version.to_owned(),
            protocol_version,
            node_uuid: self.node_uuid()?,
            boot_id,
            endpoint: path.to_path_buf(),
            pid: std::process::id(),
            socket_device,
            socket_inode,
        };
        atomic_write(
            &self.paths.descriptor_path,
            &serde_json::to_vec(&descriptor).map_err(invalid_data)?,
        )?;
        Ok(descriptor)
    }

    /// Remove a prior published socket only if its recorded inode remains at
    /// the recorded pathname. Unpublished sockets are never removed here.
    /// Only the elected owner calls this, so the descriptor's boot is dead; a
    /// device number alone may differ (see [`remove_dead_boot_socket`]).
    pub fn remove_stale_endpoint(&self) -> io::Result<()> {
        let descriptor = match parse_descriptor(&self.paths, hint_uuid(&self.paths)) {
            Ok(value) => value,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        };
        remove_dead_boot_socket(&descriptor, &self.paths.descriptor_path)?;
        remove_if_present(&self.paths.descriptor_path)
    }

    /// Remove only this election's bound socket before any descriptor was published.
    pub fn remove_unpublished_bound_socket(&self, bound: &OwnedListener) -> io::Result<()> {
        if bound.owner_token != self.owner_token || bound.path != self.paths.socket_path {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "listener does not belong to elected owner",
            ));
        }
        match fs::symlink_metadata(&self.paths.descriptor_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "endpoint descriptor was published",
                ));
            }
            Err(error) => return Err(error),
        }
        let socket = validate_socket(&bound.path)?;
        if socket.dev() != bound.socket_device || socket.ino() != bound.socket_inode {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "bound socket pathname was substituted",
            ));
        }
        fs::remove_file(&bound.path)
    }

    /// Handle publication errors that may occur after the descriptor rename.
    pub fn remove_failed_bound_publication(&self, bound: &OwnedListener) -> io::Result<()> {
        if bound.owner_token != self.owner_token || bound.path != self.paths.socket_path {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "listener does not belong to elected owner",
            ));
        }
        match parse_descriptor(&self.paths, hint_uuid(&self.paths)) {
            Ok(descriptor)
                if descriptor.boot_id == bound.boot_id
                    && descriptor.endpoint == bound.path
                    && descriptor.socket_device == bound.socket_device
                    && descriptor.socket_inode == bound.socket_inode =>
            {
                self.remove_owned_endpoint(bound.boot_id)
            }
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "published descriptor does not match bound listener",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.remove_unpublished_bound_socket(bound)
            }
            Err(error) => Err(error),
        }
    }

    /// Graceful shutdown removes only the descriptor for this boot.
    pub fn remove_owned_endpoint(&self, boot_id: Uuid) -> io::Result<()> {
        let descriptor = parse_descriptor(&self.paths, hint_uuid(&self.paths))?;
        if descriptor.boot_id != boot_id {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "endpoint boot changed",
            ));
        }
        remove_matching_socket(&descriptor, &self.paths.descriptor_path)?;
        remove_if_present(&self.paths.descriptor_path)
    }
}

pub fn read_descriptor(paths: &NodePaths, node_uuid: NodeUuid) -> io::Result<EndpointDescriptor> {
    let descriptor = parse_descriptor(paths, Some(node_uuid))?;
    validate_descriptor_socket(&descriptor, &paths.descriptor_path)?;
    Ok(descriptor)
}

/// The published socket's device or inode differs from its descriptor. The
/// text keeps the `socket pathname was substituted` prefix and names the
/// recovery that fits where it is raised (ht-tyt7).
#[derive(Debug)]
struct SocketSubstituted {
    descriptor: std::path::PathBuf,
    pid: u32,
    recovery: Recovery,
}

#[derive(Debug, Clone, Copy)]
enum Recovery {
    /// A client: the elected owner reclaims a dead boot's renumbered socket.
    Ensure,
    /// The elected owner refused it: no daemon holds the lock.
    Remove,
    /// A live process holds the owner lock and keeps it published.
    StopOwner,
}

impl std::fmt::Display for SocketSubstituted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::daemon::remedy::{RemedyContext, remedy};
        let descriptor = self.descriptor.clone();
        let context = match self.recovery {
            Recovery::Ensure => RemedyContext::SocketReclaim,
            Recovery::Remove => RemedyContext::SocketRemove { descriptor },
            Recovery::StopOwner => RemedyContext::SocketStopOwner {
                descriptor,
                pid: self.pid,
            },
        };
        write!(
            f,
            "socket pathname was substituted; {}",
            remedy(None, &context)
        )
    }
}

impl std::error::Error for SocketSubstituted {}

fn socket_substituted(
    descriptor: &EndpointDescriptor,
    descriptor_path: &Path,
    recovery: Recovery,
) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        SocketSubstituted {
            descriptor: descriptor_path.to_path_buf(),
            pid: descriptor.pid,
            recovery,
        },
    )
}

/// Whether `error` is a published socket that no longer matches its descriptor.
pub fn is_socket_substituted(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<SocketSubstituted>())
}

/// The same mismatch, restated for a live process holding the owner lock:
/// `daemon ensure` cannot reclaim it, so the text says to stop that process.
pub fn substituted_under_live_owner(error: io::Error) -> io::Error {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<SocketSubstituted>())
    {
        Some(inner) => io::Error::new(
            io::ErrorKind::PermissionDenied,
            SocketSubstituted {
                descriptor: inner.descriptor.clone(),
                pid: inner.pid,
                recovery: Recovery::StopOwner,
            },
        ),
        None => error,
    }
}

/// The node uuid named by the `node.json` hint; a missing or corrupt hint is
/// "unknown" (`Ok(None)`), as a missing namespace file was.
pub fn read_node_uuid_hint(paths: &NodePaths) -> io::Result<Option<NodeUuid>> {
    Ok(hint_uuid(paths))
}

fn hint_uuid(paths: &NodePaths) -> Option<NodeUuid> {
    read_hint(&paths.node_json_path).map(|json| json.node_uuid)
}

fn missing_hint() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "node identity hint missing")
}

/// Identity of the lock inode observed while an owner was known to hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerLockIdentity {
    device: u64,
    inode: u64,
}

fn open_existing_owner_lock(paths: &NodePaths) -> io::Result<(File, OwnerLockIdentity)> {
    let before = fs::symlink_metadata(&paths.lock_path)?;
    if !before.file_type().is_file()
        || before.uid() != effective_uid()
        || before.permissions().mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe lock file",
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&paths.lock_path)?;
    validate_private_file(&file)?;
    let opened = file.metadata()?;
    let after = fs::symlink_metadata(&paths.lock_path)?;
    if !after.file_type().is_file()
        || after.uid() != effective_uid()
        || after.permissions().mode() & 0o777 != 0o600
        || before.dev() != opened.dev()
        || before.ino() != opened.ino()
        || after.dev() != opened.dev()
        || after.ino() != opened.ino()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner lock was replaced",
        ));
    }
    Ok((
        file,
        OwnerLockIdentity {
            device: opened.dev(),
            inode: opened.ino(),
        },
    ))
}

pub fn owner_lock_identity(paths: &NodePaths) -> io::Result<OwnerLockIdentity> {
    open_existing_owner_lock(paths).map(|(_, identity)| identity)
}

/// A true result proves the same lock inode can now be exclusively leased.
/// Missing, unsafe or replaced paths cannot establish release.
pub fn previous_owner_released(paths: &NodePaths, expected: OwnerLockIdentity) -> io::Result<bool> {
    let (file, identity) = open_existing_owner_lock(paths)?;
    if identity != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner lock was replaced",
        ));
    }
    match file.try_lock() {
        Ok(()) => {
            let current = owner_lock_identity(paths)?;
            if current != expected {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "owner lock was replaced",
                ));
            }
            Ok(true)
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(err) => Err(err.into()),
    }
}

/// `expected` is the hint's uuid; it is compared only when the hint is known.
fn parse_descriptor(
    paths: &NodePaths,
    expected: Option<NodeUuid>,
) -> io::Result<EndpointDescriptor> {
    let bytes = read_private(&paths.descriptor_path)?;
    let descriptor: EndpointDescriptor = serde_json::from_slice(&bytes).map_err(invalid_data)?;
    if expected.is_some_and(|expected| descriptor.node_uuid != expected)
        || !paths.accepts_descriptor_endpoint(&descriptor.endpoint)
        || descriptor.software_version.is_empty()
        || descriptor.protocol_version == 0
        || descriptor.boot_id.is_nil()
        || descriptor.socket_inode == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "endpoint descriptor mismatch",
        ));
    }
    Ok(descriptor)
}

fn validate_descriptor_socket(
    descriptor: &EndpointDescriptor,
    descriptor_path: &Path,
) -> io::Result<()> {
    let meta = validate_socket(&descriptor.endpoint)?;
    if meta.dev() != descriptor.socket_device || meta.ino() != descriptor.socket_inode {
        return Err(socket_substituted(
            descriptor,
            descriptor_path,
            Recovery::Ensure,
        ));
    }
    Ok(())
}

/// [`remove_matching_socket`] for the elected owner retiring a dead boot's
/// endpoint. macOS renumbers an APFS volume's `st_dev` across reboots while
/// the socket keeps its inode (ht-tyt7), so a socket whose inode matches but
/// whose device alone differs is that dead boot's socket and is removed once
/// it accepts no connection. The owner lock proves no live owner of this
/// instance holds it; the refused connect proves nothing else listens; the
/// lstat before and after (a same-uid 0600 socket, never a symlink, with an
/// unchanged device and inode) proves the unlinked file is the one checked.
/// An inode mismatch is not explained by a renumbered mount and stays
/// refused, as does a socket that answers.
fn remove_dead_boot_socket(
    descriptor: &EndpointDescriptor,
    descriptor_path: &Path,
) -> io::Result<()> {
    remove_dead_boot_socket_as(descriptor, descriptor_path, effective_uid())
}

/// [`remove_dead_boot_socket`] with the owning uid supplied, so a test can
/// show that a socket of another uid is refused on any host.
fn remove_dead_boot_socket_as(
    descriptor: &EndpointDescriptor,
    descriptor_path: &Path,
    uid: u32,
) -> io::Result<()> {
    let substituted = || socket_substituted(descriptor, descriptor_path, Recovery::Remove);
    let meta = match validate_socket_as(&descriptor.endpoint, uid) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if meta.ino() != descriptor.socket_inode {
        return Err(substituted());
    }
    if meta.dev() != descriptor.socket_device {
        match std::os::unix::net::UnixStream::connect(&descriptor.endpoint) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "a live listener holds the daemon socket pathname without the owner lock",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
            Err(error) => return Err(error),
        }
        let again = validate_socket_as(&descriptor.endpoint, uid)?;
        if again.dev() != meta.dev() || again.ino() != meta.ino() {
            return Err(substituted());
        }
    }
    fs::remove_file(&descriptor.endpoint)
}

/// Clear the stable pathname before binding. Only the elected owner calls
/// this, and the exclusive lock proves no live owner of this instance holds
/// a listener there, so a socket left at the pathname is from a dead boot
/// (for example one that crashed between bind and publication). It is
/// removed only when it is a socket owned by the effective user that accepts
/// no connection; anything else (a symlink, a regular file, a directory, a
/// foreign socket or a live listener) is refused and left untouched.
fn reclaim_dead_socket(path: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !meta.file_type().is_socket() || meta.uid() != effective_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe artifact at daemon socket pathname",
        ));
    }
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "a live listener holds the daemon socket pathname without the owner lock",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
        Err(error) => return Err(error),
    }
    let again = fs::symlink_metadata(path)?;
    if !again.file_type().is_socket() || again.dev() != meta.dev() || again.ino() != meta.ino() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon socket pathname was substituted",
        ));
    }
    fs::remove_file(path)
}

fn remove_matching_socket(
    descriptor: &EndpointDescriptor,
    descriptor_path: &Path,
) -> io::Result<()> {
    match validate_descriptor_socket(descriptor, descriptor_path) {
        Ok(()) => fs::remove_file(&descriptor.endpoint),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn open_private_file(path: &Path) -> io::Result<File> {
    if let Ok(meta) = fs::symlink_metadata(path)
        && (!meta.file_type().is_file()
            || meta.uid() != effective_uid()
            || meta.permissions().mode() & 0o777 != 0o600)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe lock file",
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        // A lock file: never truncate an existing one (the default, made explicit).
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    validate_private_file(&file)?;
    Ok(file)
}

fn read_private(path: &Path) -> io::Result<Vec<u8>> {
    let before = fs::symlink_metadata(path)?;
    if !before.file_type().is_file()
        || before.uid() != effective_uid()
        || before.permissions().mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe private file",
        ));
    }
    let file = File::open(path)?;
    validate_private_file(&file)?;
    let opened = file.metadata()?;
    let after = fs::symlink_metadata(path)?;
    if !after.file_type().is_file()
        || after.uid() != effective_uid()
        || after.permissions().mode() & 0o777 != 0o600
        || before.dev() != opened.dev()
        || before.ino() != opened.ino()
        || after.dev() != opened.dev()
        || after.ino() != opened.ino()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private file was replaced",
        ));
    }
    let mut bytes = Vec::new();
    file.take(8193).read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private file too large",
        ));
    }
    Ok(bytes)
}

fn validate_private_file(file: &File) -> io::Result<()> {
    let meta = file.metadata()?;
    if !meta.file_type().is_file()
        || meta.uid() != effective_uid()
        || meta.permissions().mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe private file",
        ));
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.durable_sync()?;
        fs::rename(&temporary, path)?;
        File::open(path.parent().unwrap())?.durable_sync()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn validate_socket(path: &Path) -> io::Result<fs::Metadata> {
    validate_socket_as(path, effective_uid())
}

fn validate_socket_as(path: &Path, uid: u32) -> io::Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_socket()
        || meta.uid() != uid
        || meta.permissions().mode() & 0o777 != 0o600
    {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe socket artifact",
        ))
    } else {
        Ok(meta)
    }
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
#[path = "../../tests/daemon/ownership.rs"]
mod tests;
