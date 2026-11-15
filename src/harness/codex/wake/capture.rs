use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FileIdentity {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Descriptor {
    pub executable: FileIdentity,
    pub database: FileIdentity,
    pub session_lock: FileIdentity,
}
pub(super) fn files(process: &Path, session: &str, deadline: Instant) -> Option<Descriptor> {
    use std::{collections::BTreeMap, fs, io::Read, os::unix::fs::MetadataExt};
    if uuid::Uuid::parse_str(session).ok()?.to_string() != session {
        return None;
    }
    // Bound reads, discard unrelated environment values, retain no argv/environment dump.
    for (name, limit) in [("cmdline", 32768), ("environ", 65536)] {
        let mut bytes = Vec::new();
        fs::File::open(process.join(name))
            .ok()?
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > limit as usize || Instant::now() >= deadline {
            return None;
        }
        if name == "cmdline" && bytes.split(|b| *b == 0).any(|v| v == b"--managed-daemon") {
            return None;
        }
    }
    let executable = from_fd(&process.join("exe"))?;
    let mut databases = BTreeMap::new();
    let mut locks = BTreeMap::new();
    let wanted = format!("{session}.lock");
    for (count, entry) in fs::read_dir(process.join("fd")).ok()?.enumerate() {
        if count >= 1024 || Instant::now() >= deadline {
            return None;
        }
        let entry = entry.ok()?;
        let path = fs::read_link(entry.path()).ok()?;
        if !path.is_absolute() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // A superseded/deleted connection can still be the watcher: never silently ignore it.
        if (name.starts_with("queue_")
            && (name.ends_with(".sqlite") || name.ends_with(".sqlite (deleted)"))
            && name != "queue_1.sqlite")
            || name == format!("{wanted} (deleted)")
        {
            return None;
        }
        let queue = name == "queue_1.sqlite";
        let lock = name == wanted && path.parent()?.file_name()?.to_str()? == "thread-writer-locks";
        if !queue && !lock {
            continue;
        }
        let observed = from_fd(&entry.path())?;
        // Verify the FD and its current path name still identify the same regular file.
        let fd_meta = fs::metadata(entry.path()).ok()?;
        if (fd_meta.dev(), fd_meta.ino()) != (observed.device, observed.inode) {
            return None;
        }
        let key = (observed.device, observed.inode);
        if queue {
            databases.insert(key, observed);
        } else {
            locks.insert(key, observed);
        }
    }
    if databases.len() != 1 || locks.len() != 1 || Instant::now() >= deadline {
        return None;
    }
    Some(Descriptor {
        executable,
        database: databases.into_values().next()?,
        session_lock: locks.into_values().next()?,
    })
}
pub(super) fn from_fd(link: &Path) -> Option<FileIdentity> {
    let path = std::fs::read_link(link).ok()?;
    if !path.is_absolute()
        || path.as_os_str().len() > 4096
        || path.to_str()?.ends_with(" (deleted)")
    {
        return None;
    }
    let identity = identity(&path)?;
    use std::os::unix::fs::MetadataExt;
    let fd = std::fs::metadata(link).ok()?;
    (fd.is_file() && (fd.dev(), fd.ino()) == (identity.device, identity.inode)).then_some(identity)
}
pub(super) fn identity(path: &Path) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    meta.is_file().then(|| FileIdentity {
        path: path.to_path_buf(),
        device: meta.dev(),
        inode: meta.ino(),
    })
}

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../../tests/harness/codex_wake_capture.rs"]
mod tests;
