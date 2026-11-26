//! Pure selection rules over a libproc-style vnode view: a path the kernel name cache
//! reports plus the identity of the vnode itself. The name can lag a rename, follow a
//! hardlink or lose its directory, so every name ambiguity fails closed instead of being
//! skipped. Nothing here reads the system; callers supply the current path identity.
use super::capture::FileIdentity;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Vnode {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
    pub nlink: u64,
    pub regular: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Region {
    pub vnode: Vnode,
    pub executable: bool,
}

/// The vnode's identity when its reported name is absolute, singly linked and still
/// names that same regular file. Unlinked (nlink 0), replaced or hardlinked files fail.
fn live(v: &Vnode, current: &dyn Fn(&Path) -> Option<FileIdentity>) -> Option<FileIdentity> {
    if !v.path.is_absolute() || !v.regular || v.nlink != 1 {
        return None;
    }
    let now = current(&v.path)?;
    (now.device == v.device && now.inode == v.inode).then_some(now)
}
fn queue_family(name: &str) -> bool {
    name.starts_with("queue_") && name.ends_with(".sqlite")
}

/// Exactly one live queue_1.sqlite and one live session writer lock, or None.
pub(super) fn receiver_files(
    files: &[Vnode],
    session: &str,
    current: &dyn Fn(&Path) -> Option<FileIdentity>,
) -> Option<(FileIdentity, FileIdentity)> {
    if uuid::Uuid::parse_str(session).ok()?.to_string() != session {
        return None;
    }
    let wanted = format!("{session}.lock");
    let mut databases = BTreeMap::new();
    let mut locks = BTreeMap::new();
    for v in files {
        let Some(name) = v.path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let family = queue_family(name);
        if !family && name != wanted {
            continue;
        }
        // Another queue database, or any ambiguous name of a candidate, makes the
        // receiver's storage scope unknown; Linux reads these as "(deleted)" links.
        if family && name != "queue_1.sqlite" {
            return None;
        }
        let identity = live(v, current)?;
        let key = (identity.device, identity.inode);
        if family {
            databases.insert(key, identity);
        } else if v.path.parent()?.file_name()?.to_str()? == "thread-writer-locks" {
            locks.insert(key, identity);
        }
    }
    if databases.len() != 1 || locks.len() != 1 {
        return None;
    }
    Some((databases.into_values().next()?, locks.into_values().next()?))
}

/// The main image identity without a public text-vnode API: the process path `path`,
/// its current identity before and after a complete region walk, and every file-backed
/// region carrying that path. All such regions must be that same live file and one of
/// them executable. A replaced path (a region with the old inode), an unlinked image, an
/// incomplete walk or a changed path refuse; regions of other files never select.
pub(super) fn executable(
    path: &Path,
    before: Option<FileIdentity>,
    regions: &[Region],
    complete: bool,
    after_path: Option<&Path>,
    after: Option<FileIdentity>,
) -> Option<FileIdentity> {
    let before = before?;
    if !complete || after_path != Some(path) || after.as_ref() != Some(&before) {
        return None;
    }
    let mut mapped = regions.iter().filter(|r| r.vnode.path == path).peekable();
    mapped.peek()?;
    let mut executable = false;
    for region in mapped {
        let v = &region.vnode;
        if (v.device, v.inode) != (before.device, before.inode) || v.nlink == 0 || !v.regular {
            return None;
        }
        executable |= region.executable;
    }
    executable.then_some(before)
}

/// Identities of the helper's queue_1.sqlite connections; any other or ambiguous queue
/// database is an error, never a skip.
pub(super) fn helper_databases(
    files: &[Vnode],
    current: &dyn Fn(&Path) -> Option<FileIdentity>,
) -> Result<std::collections::BTreeSet<(u64, u64)>, &'static str> {
    let mut found = std::collections::BTreeSet::new();
    for v in files {
        let Some(name) = v.path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !queue_family(name) {
            continue;
        }
        if name != "queue_1.sqlite" || !v.path.is_absolute() {
            return Err("queue helper database ambiguous");
        }
        let identity = live(v, current).ok_or("queue helper database changed")?;
        found.insert((identity.device, identity.inode));
    }
    Ok(found)
}

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../../tests/harness/codex_wake_rules.rs"]
mod tests;
