//! `node.json`: the derived diagnostic hint file next to `node.key`.
//!
//! Owns: the `format:1` shape, the regenerate-if-differs atomic write (0600)
//! and a reader that returns `Option` (missing or corrupt means "identity
//! unknown"). Hides: the file format, and that the file is never
//! authoritative. Must not: be read by the elected daemon as a source of uuid
//! or tag (only the store's `local_node` row and the key hold those), or be
//! created anywhere but by the daemon after the store opens or after a retag
//! applies. Readers (descriptor validation, doctor, `peer --stdio`
//! diagnostics) treat it as a hint. New diagnostic fields go here.
use std::{
    fs,
    fs::{File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

use serde::{Deserialize, Serialize};

use super::tag::{NodeTag, NodeUuid};

/// File name inside the node directory.
pub const NODE_JSON_FILE: &str = "node.json";
const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeJson {
    pub format: u32,
    pub node_uuid: NodeUuid,
    pub tag: NodeTag,
    pub tag_k: u32,
    pub created_at: i64,
}

impl NodeJson {
    pub fn new(node_uuid: NodeUuid, tag: NodeTag, tag_k: u32, created_at: i64) -> Self {
        Self {
            format: FORMAT,
            node_uuid,
            tag,
            tag_k,
            created_at,
        }
    }
}

/// The hint at `path`; missing, unreadable, unparsable or another format is
/// `None`.
pub fn read_hint(path: &Path) -> Option<NodeJson> {
    let bytes = fs::read(path).ok()?;
    let parsed: NodeJson = serde_json::from_slice(&bytes).ok()?;
    (parsed.format == FORMAT).then_some(parsed)
}

/// Write `want` unless the file already holds it with mode 0600. Returns
/// whether it wrote.
pub fn write_if_differs(path: &Path, want: &NodeJson) -> io::Result<bool> {
    if read_hint(path).as_ref() == Some(want)
        && fs::metadata(path).is_ok_and(|m| m.mode() & 0o777 == 0o600)
    {
        return Ok(false);
    }
    let mut bytes = serde_json::to_vec(want).map_err(io::Error::other)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)?;
    Ok(true)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(format!(
        "json.tmp.{}.{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        match path.parent() {
            Some(parent) => File::open(parent)?.sync_all(),
            None => Ok(()),
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::{node::tag::tag_for, test_support::isolation::TestIsolation};

    fn want(k: u32) -> NodeJson {
        let uuid = NodeUuid::from_bytes([4; 16]);
        NodeJson::new(uuid, tag_for(uuid, k), k, 1_700_000_000)
    }

    #[test]
    fn write_if_differs_creates_missing_file_0600() {
        let iso = TestIsolation::new("nodejson");
        let path = iso.state_root().join(NODE_JSON_FILE);
        assert!(write_if_differs(&path, &want(0)).unwrap());
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(read_hint(&path), Some(want(0)));
        let text = fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["format"], 1);
        assert_eq!(value["node_uuid"], "04040404040404040404040404040404");
        assert_eq!(value["tag_k"], 0);
        assert_eq!(value["created_at"], 1_700_000_000);
        assert_eq!(value["tag"], want(0).tag.as_str());
    }

    #[test]
    fn rewrites_corrupt_file() {
        let iso = TestIsolation::new("nodejson");
        let path = iso.state_root().join(NODE_JSON_FILE);
        fs::write(&path, b"\xff\x00garbage").unwrap();
        assert_eq!(read_hint(&path), None);
        assert!(write_if_differs(&path, &want(0)).unwrap());
        assert_eq!(read_hint(&path), Some(want(0)));
    }

    #[test]
    fn rewrites_stale_tag() {
        let iso = TestIsolation::new("nodejson");
        let path = iso.state_root().join(NODE_JSON_FILE);
        assert!(write_if_differs(&path, &want(0)).unwrap());
        assert!(write_if_differs(&path, &want(1)).unwrap());
        assert_eq!(read_hint(&path), Some(want(1)));
    }

    #[test]
    fn rewrites_wrong_mode() {
        let iso = TestIsolation::new("nodejson");
        let path = iso.state_root().join(NODE_JSON_FILE);
        write_if_differs(&path, &want(0)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(write_if_differs(&path, &want(0)).unwrap());
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn unchanged_file_is_not_rewritten() {
        let iso = TestIsolation::new("nodejson");
        let path = iso.state_root().join(NODE_JSON_FILE);
        write_if_differs(&path, &want(0)).unwrap();
        let before = fs::metadata(&path).unwrap();
        assert!(!write_if_differs(&path, &want(0)).unwrap());
        let after = fs::metadata(&path).unwrap();
        assert_eq!(before.ino(), after.ino());
        assert_eq!(before.mtime_nsec(), after.mtime_nsec());
        assert_eq!(before.mtime(), after.mtime());
    }

    #[test]
    fn corrupt_reader_returns_none() {
        let iso = TestIsolation::new("nodejson");
        let path = iso.state_root().join(NODE_JSON_FILE);
        assert_eq!(read_hint(&path), None);
        fs::write(&path, b"not json").unwrap();
        assert_eq!(read_hint(&path), None);
        let mut wrong = want(0);
        wrong.format = 2;
        fs::write(&path, serde_json::to_vec(&wrong).unwrap()).unwrap();
        assert_eq!(read_hint(&path), None);
        fs::write(&path, br#"{"format":1,"node_uuid":"zz"}"#).unwrap();
        assert_eq!(read_hint(&path), None);
    }
}
