use std::{
    fmt, fs,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
};

use ed25519_dalek::{Signer, SigningKey};

use crate::daemon::paths::NodePaths;

use super::tag::{NodeTag, NodeUuid, tag_for};

/// File name of the node's signing seed inside the node directory.
pub const KEY_FILE: &str = "node.key";
const SEED_LEN: u64 = 32;

/// Why the node key could not be used. Every variant names the key path; the
/// daemon refuses to start on any of them and never repairs the key.
#[derive(Debug)]
pub enum NodeKeyError {
    KeyMissingForStore {
        path: PathBuf,
    },
    Unreadable {
        path: PathBuf,
        source: io::Error,
    },
    Permissions {
        path: PathBuf,
        mode: u32,
        uid: u32,
    },
    NotRegularFile {
        path: PathBuf,
    },
    Malformed {
        path: PathBuf,
        len: u64,
    },
    /// Raised by the daemon start sequence when the store's node row and the
    /// key disagree; never by this module's own loading.
    IdentityMismatch {
        path: PathBuf,
        store: NodeUuid,
        key: NodeUuid,
    },
}

impl NodeKeyError {
    /// The variant's name, for one-line start-failure text.
    pub fn variant(&self) -> &'static str {
        match self {
            Self::KeyMissingForStore { .. } => "KeyMissingForStore",
            Self::Unreadable { .. } => "Unreadable",
            Self::Permissions { .. } => "Permissions",
            Self::NotRegularFile { .. } => "NotRegularFile",
            Self::Malformed { .. } => "Malformed",
            Self::IdentityMismatch { .. } => "IdentityMismatch",
        }
    }
}

impl fmt::Display for NodeKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyMissingForStore { path } => write!(
                f,
                "node key {} is missing but the store exists: restore node.key, or move herdr-threads.db away to start as a new node",
                path.display()
            ),
            Self::Unreadable { path, source } => {
                write!(f, "node key {} is unreadable: {source}", path.display())
            }
            Self::Permissions { path, mode, uid } => write!(
                f,
                "node key {} has unsafe permissions (mode {:o}, owner uid {uid}): must be 0600 and owned by this user",
                path.display(),
                mode & 0o7777
            ),
            Self::NotRegularFile { path } => {
                write!(f, "node key {} is not a regular file", path.display())
            }
            Self::Malformed { path, len } => write!(
                f,
                "node key {} is malformed: {len} bytes, expected {SEED_LEN}",
                path.display()
            ),
            Self::IdentityMismatch { path, store, key } => write!(
                f,
                "node key {} belongs to node {key} but the store belongs to node {store}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for NodeKeyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The node's key custody and identity.
///
/// Owns: loading or creating `node.key` (0600, created once, never
/// rewritten), deriving the `node_uuid`, signing hashes, the current tag and
/// the retag counter `tag_k`. Hides: Ed25519 and the tag derivation. Must
/// not: rewrite or regenerate a key it cannot read (it refuses), verify other
/// nodes' entries (the envelope does), or be the tag record (the store's
/// `local_node` row is; minting reads the row, never this cache). Where new
/// behaviour goes: tag derivation and the retag rule belong in `tag.rs`.
/// Role: one `Arc<NodeIdentity>` per daemon; no trait, no global.
pub struct NodeIdentity {
    uuid: NodeUuid,
    signing: SigningKey,
    public: [u8; 32],
    tag_k: AtomicU32,
}

impl fmt::Debug for NodeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeIdentity")
            .field("uuid", &self.uuid)
            .field("tag_k", &self.tag_k())
            .finish_non_exhaustive()
    }
}

impl NodeIdentity {
    pub(crate) fn from_seed(seed: [u8; 32]) -> Self {
        let signing = SigningKey::from_bytes(&seed);
        let public = signing.verifying_key().to_bytes();
        Self {
            uuid: NodeUuid::from_public_key(&public),
            signing,
            public,
            tag_k: AtomicU32::new(0),
        }
    }

    /// Load `<node_dir>/node.key`, or create it when it is missing and the
    /// store does not exist yet. A missing key next to an existing store, and
    /// every unsafe or malformed key, is refused. The caller creates
    /// `node_dir`; a missing directory surfaces as `Unreadable`.
    pub fn load_or_create(paths: &NodePaths, store_exists: bool) -> Result<Self, NodeKeyError> {
        Self::load_or_create_in(&paths.node_dir, store_exists)
    }

    /// [`Self::load_or_create`] over a bare node directory (the unit tests').
    pub(crate) fn load_or_create_in(
        node_dir: &Path,
        store_exists: bool,
    ) -> Result<Self, NodeKeyError> {
        let key_path = node_dir.join(KEY_FILE);
        match read_seed(&key_path)? {
            Some(seed) => Ok(Self::from_seed(seed)),
            None if store_exists => Err(NodeKeyError::KeyMissingForStore { path: key_path }),
            None => create_key(node_dir, &key_path, || {}),
        }
    }

    pub fn uuid(&self) -> NodeUuid {
        self.uuid
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.public
    }

    pub fn sign(&self, hash: &[u8; 32]) -> [u8; 64] {
        self.signing.sign(hash).to_bytes()
    }

    pub fn tag_k(&self) -> u32 {
        self.tag_k.load(Ordering::SeqCst)
    }

    pub fn tag(&self) -> NodeTag {
        tag_for(self.uuid, self.tag_k())
    }

    pub fn set_tag_k(&self, k: u32) {
        self.tag_k.store(k, Ordering::SeqCst);
    }
}

fn unreadable(path: &Path, source: io::Error) -> NodeKeyError {
    NodeKeyError::Unreadable {
        path: path.to_path_buf(),
        source,
    }
}

/// `Ok(None)` when the key file does not exist.
fn read_seed(path: &Path) -> Result<Option<[u8; 32]>, NodeKeyError> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) if err.raw_os_error() == Some(libc::ELOOP) => {
            return Err(NodeKeyError::NotRegularFile {
                path: path.to_path_buf(),
            });
        }
        Err(err) => return Err(unreadable(path, err)),
    };
    let meta = file.metadata().map_err(|e| unreadable(path, e))?;
    if !meta.is_file() {
        return Err(NodeKeyError::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    if meta.mode() & 0o077 != 0 || meta.uid() != euid {
        return Err(NodeKeyError::Permissions {
            path: path.to_path_buf(),
            mode: meta.mode() & 0o7777,
            uid: meta.uid(),
        });
    }
    if meta.len() != SEED_LEN {
        return Err(NodeKeyError::Malformed {
            path: path.to_path_buf(),
            len: meta.len(),
        });
    }
    let mut seed = [0u8; 32];
    file.read_exact(&mut seed)
        .map_err(|e| unreadable(path, e))?;
    Ok(Some(seed))
}

/// Write a fresh seed to a temp file and `link()` it into place without
/// clobbering. `before_link` runs between the temp write and the link (the
/// race test places a competing key there); production passes `|| {}`.
fn create_key(
    node_dir: &Path,
    key_path: &Path,
    before_link: impl FnOnce(),
) -> Result<NodeIdentity, NodeKeyError> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| unreadable(key_path, io::Error::other(e)))?;
    let temp = node_dir.join(format!("node.key.tmp.{}", std::process::id()));
    let write_temp = || -> io::Result<()> {
        let open = || {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_CLOEXEC)
                .open(&temp)
        };
        let mut file = match open() {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                // A crashed earlier daemon with the same pid left it.
                fs::remove_file(&temp)?;
                open()?
            }
            Err(err) => return Err(err),
        };
        file.write_all(&seed)?;
        file.sync_all()
    };
    write_temp().map_err(|e| {
        let _ = fs::remove_file(&temp);
        unreadable(key_path, e)
    })?;
    before_link();
    let linked = fs::hard_link(&temp, key_path);
    let _ = fs::remove_file(&temp);
    match linked {
        Ok(()) => {
            File::open(node_dir)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| unreadable(key_path, e))?;
            Ok(NodeIdentity::from_seed(seed))
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            // Another creator won: use its key, never overwrite it.
            match read_seed(key_path)? {
                Some(winner) => Ok(NodeIdentity::from_seed(winner)),
                None => Err(unreadable(key_path, err)),
            }
        }
        Err(err) => Err(unreadable(key_path, err)),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::test_support::isolation::TestIsolation;

    fn put_key(dir: &Path, bytes: &[u8], mode: u32) -> PathBuf {
        let path = dir.join(KEY_FILE);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn create_then_reload_same_uuid() {
        let iso = TestIsolation::new("node");
        let first = NodeIdentity::load_or_create_in(iso.state_root(), false).unwrap();
        let second = NodeIdentity::load_or_create_in(iso.state_root(), true).unwrap();
        assert_eq!(first.uuid(), second.uuid());
        assert_eq!(first.public_key(), second.public_key());
    }

    #[test]
    fn created_key_is_0600_and_32_bytes() {
        let iso = TestIsolation::new("node");
        NodeIdentity::load_or_create_in(iso.state_root(), false).unwrap();
        let meta = fs::metadata(iso.state_root().join(KEY_FILE)).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(meta.len(), 32);
        let leftovers: Vec<_> = fs::read_dir(iso.state_root())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("node.key.tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn refuses_group_readable_key() {
        let iso = TestIsolation::new("node");
        let path = put_key(iso.state_root(), &[3u8; 32], 0o644);
        match NodeIdentity::load_or_create_in(iso.state_root(), true) {
            Err(NodeKeyError::Permissions { mode, path: p, .. }) => {
                assert_eq!(mode & 0o777, 0o644);
                assert_eq!(p, path);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(fs::read(&path).unwrap(), vec![3u8; 32]);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn refuses_malformed_lengths() {
        for len in [31usize, 33] {
            let iso = TestIsolation::new("node");
            put_key(iso.state_root(), &vec![5u8; len], 0o600);
            match NodeIdentity::load_or_create_in(iso.state_root(), true) {
                Err(NodeKeyError::Malformed { len: got, .. }) => assert_eq!(got, len as u64),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn refuses_directory_and_symlink() {
        let iso = TestIsolation::new("node");
        fs::create_dir(iso.state_root().join(KEY_FILE)).unwrap();
        assert!(matches!(
            NodeIdentity::load_or_create_in(iso.state_root(), true),
            Err(NodeKeyError::NotRegularFile { .. })
        ));

        let iso = TestIsolation::new("node");
        let real = iso.state_root().join("real.key");
        fs::write(&real, [1u8; 32]).unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&real, iso.state_root().join(KEY_FILE)).unwrap();
        assert!(matches!(
            NodeIdentity::load_or_create_in(iso.state_root(), true),
            Err(NodeKeyError::NotRegularFile { .. })
        ));
    }

    #[test]
    fn missing_key_with_store_refuses() {
        let iso = TestIsolation::new("node");
        let err = NodeIdentity::load_or_create_in(iso.state_root(), true).unwrap_err();
        assert!(matches!(err, NodeKeyError::KeyMissingForStore { .. }));
        assert!(err.to_string().contains("restore node.key"));
        assert!(!iso.state_root().join(KEY_FILE).exists());
    }

    #[test]
    fn missing_directory_is_unreadable() {
        let iso = TestIsolation::new("node");
        let err = NodeIdentity::load_or_create_in(&iso.state_root().join("absent"), false);
        assert!(matches!(err, Err(NodeKeyError::Unreadable { .. })));
    }

    #[test]
    fn race_keeps_pre_placed_key() {
        let iso = TestIsolation::new("node");
        let dir = iso.state_root().to_path_buf();
        let key_path = dir.join(KEY_FILE);
        let placed = [0x42u8; 32];
        let id = create_key(&dir, &key_path, || {
            put_key(&dir, &placed, 0o600);
        })
        .unwrap();
        assert_eq!(id.uuid(), NodeIdentity::from_seed(placed).uuid());
        assert_eq!(fs::read(&key_path).unwrap(), placed);
        assert!(
            !dir.join(format!("node.key.tmp.{}", std::process::id()))
                .exists()
        );
    }

    #[test]
    fn fixed_seed_golden_node_uuid() {
        let iso = TestIsolation::new("node");
        put_key(iso.state_root(), &[7u8; 32], 0o600);
        let id = NodeIdentity::load_or_create_in(iso.state_root(), true).unwrap();
        let public = SigningKey::from_bytes(&[7u8; 32])
            .verifying_key()
            .to_bytes();
        let independent = Sha256::digest(public);
        let hex: String = independent[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(id.uuid().to_string(), hex);
        assert_eq!(id.uuid().to_string(), "fe812c12f3ab4ce6ac5db69ac352f906");
    }

    #[test]
    fn sign_verifies_with_verifying_key() {
        let id = NodeIdentity::from_seed([9u8; 32]);
        let hash = [0xabu8; 32];
        let sig = Signature::from_bytes(&id.sign(&hash));
        let key = VerifyingKey::from_bytes(&id.public_key()).unwrap();
        assert!(key.verify(&hash, &sig).is_ok());
        let mut flipped = hash;
        flipped[0] ^= 1;
        assert!(key.verify(&flipped, &sig).is_err());
    }

    #[test]
    fn tag_follows_set_tag_k() {
        let id = NodeIdentity::from_seed([1u8; 32]);
        assert_eq!(id.tag_k(), 0);
        assert_eq!(id.tag(), tag_for(id.uuid(), 0));
        id.set_tag_k(2);
        assert_eq!(id.tag_k(), 2);
        assert_eq!(id.tag(), tag_for(id.uuid(), 2));
        assert!(!format!("{id:?}").contains("signing"));
    }
}
