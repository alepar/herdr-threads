//! `<state>/instances`, the pre-node-layout data tree: a read-only census for
//! `doctor`, and the operator-gated `legacy purge`.
//!
//! LegacyInventory owns every access to `<state>/instances`: the path (only
//! from `paths::legacy_instances_dir`), the instance count, the recursive
//! byte size (a `symlink_metadata` walk that never follows links), the
//! `settings.json` paths, the live owners (a non-blocking `flock` try on each
//! `owner.lock`, released at once, the pid read from that instance's
//! `endpoint.json`), the one-line texts, and the purge preconditions (agent
//! evidence without `--operator`, a symlink or non-directory, a directory not
//! owned by the effective uid, a held lock). The only removal is the
//! `remove_dir_all` in [`purge`].
//!
//! Never: open, migrate or copy a legacy database or settings file; touch
//! `node/`; be reachable from the daemon. New legacy findings go here and
//! nothing else, and never the daemon, opens or removes `instances/`.

use serde_json::{Value, json};
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// The remedy doctor and the dry run name.
pub(crate) const REMEDY: &str = "herdr-threads legacy purge";

/// Largest `endpoint.json` read for a pid.
const DESCRIPTOR_READ_LIMIT: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Shape {
    Directory,
    Symlink,
    NotDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveOwner {
    /// The instance directory's name.
    pub(crate) instance: String,
    pub(crate) pid: Option<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct LegacyInventory {
    pub(crate) path: PathBuf,
    pub(crate) shape: Shape,
    owner_uid: u32,
    pub(crate) instances: usize,
    pub(crate) bytes: u64,
    pub(crate) settings: Vec<PathBuf>,
    pub(crate) live: Vec<LiveOwner>,
}

/// Why `legacy purge --confirm` refused; each is one line, exit 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PurgeRefusal {
    AgentEvidence(String),
    Symlink,
    NotDirectory,
    NotOwned { uid: u32 },
    Live { pid: Option<u32> },
}

impl std::fmt::Display for PurgeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AgentEvidence(evidence) => write!(
                f,
                "legacy purge --confirm never runs as an agent ({evidence}); run it in your own \
                 shell, or override as the local account with `herdr-threads legacy purge \
                 --confirm --operator`"
            ),
            Self::Symlink => f.write_str("instances is a symlink; not purging"),
            Self::NotDirectory => f.write_str("instances is not a directory; not purging"),
            Self::NotOwned { uid } => write!(f, "instances not owned by uid {uid}; not purging"),
            Self::Live { pid } => match pid {
                Some(pid) => write!(f, "legacy daemon running: pid {pid}; stop it first"),
                None => f.write_str("legacy daemon running: pid unknown; stop it first"),
            },
        }
    }
}

impl LegacyInventory {
    /// The census of `<state_dir>/instances`; `None` when it is absent or
    /// unreadable. Never fails on contents: unreadable parts count as zero.
    pub(crate) fn scan(state_dir: &Path) -> Option<Self> {
        let path = crate::daemon::paths::legacy_instances_dir(state_dir);
        let meta = fs::symlink_metadata(&path).ok()?;
        let shape = if meta.file_type().is_symlink() {
            Shape::Symlink
        } else if meta.is_dir() {
            Shape::Directory
        } else {
            Shape::NotDirectory
        };
        let mut inventory = Self {
            path,
            shape,
            owner_uid: meta.uid(),
            instances: 0,
            bytes: 0,
            settings: Vec::new(),
            live: Vec::new(),
        };
        if inventory.shape != Shape::Directory {
            return Some(inventory);
        }
        inventory.bytes = tree_bytes(&inventory.path);
        let mut dirs: Vec<PathBuf> = fs::read_dir(&inventory.path)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        dirs.sort();
        inventory.instances = dirs.len();
        for dir in dirs {
            let settings = dir.join("settings.json");
            if fs::symlink_metadata(&settings).is_ok_and(|meta| meta.is_file()) {
                inventory.settings.push(settings);
            }
            if let Some(pid) = live_owner(&dir) {
                inventory.live.push(LiveOwner {
                    instance: dir
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    pid,
                });
            }
        }
        Some(inventory)
    }

    /// The doctor `legacy` object.
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "path": self.path.display().to_string(),
            "instances": self.instances,
            "bytes": self.bytes,
            "settings": self.settings.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "live": self.live.iter().map(|o| json!({"instance": o.instance, "pid": o.pid})).collect::<Vec<_>>(),
            "remedy": REMEDY,
        })
    }

    /// Why a purge must not proceed, in the order the operator can fix them.
    pub(crate) fn purge_preconditions(
        &self,
        agent_evidence: Option<String>,
        operator: bool,
        euid: u32,
    ) -> Result<(), PurgeRefusal> {
        if !operator && let Some(evidence) = agent_evidence {
            return Err(PurgeRefusal::AgentEvidence(evidence));
        }
        match self.shape {
            Shape::Symlink => return Err(PurgeRefusal::Symlink),
            Shape::NotDirectory => return Err(PurgeRefusal::NotDirectory),
            Shape::Directory => {}
        }
        if self.owner_uid != euid {
            return Err(PurgeRefusal::NotOwned { uid: euid });
        }
        if let Some(owner) = self.live.first() {
            return Err(PurgeRefusal::Live { pid: owner.pid });
        }
        Ok(())
    }
}

fn line_for(path: &str, instances: usize, bytes: u64) -> String {
    format!(
        "legacy data: {path} ({instances} instances, {}) not used; remove: {REMEDY}",
        human_size(bytes)
    )
}

/// Doctor's text lines for a report's `legacy` object; empty when absent.
pub(crate) fn report_lines(legacy: &Value) -> String {
    let Some(path) = legacy["path"].as_str() else {
        return String::new();
    };
    let mut out = line_for(
        path,
        legacy["instances"].as_u64().unwrap_or(0) as usize,
        legacy["bytes"].as_u64().unwrap_or(0),
    );
    out.push('\n');
    for owner in legacy["live"].as_array().into_iter().flatten() {
        match owner["pid"].as_u64() {
            Some(pid) => out.push_str(&format!("legacy daemon running: pid {pid}\n")),
            None => out.push_str("legacy daemon running: pid unknown\n"),
        }
    }
    for settings in legacy["settings"].as_array().into_iter().flatten() {
        if let Some(settings) = settings.as_str() {
            out.push_str(&format!("legacy settings not carried over: {settings}\n"));
        }
    }
    out
}

/// Remove `<state>/instances` (the only removal of the legacy tree).
pub(crate) fn purge(inventory: &LegacyInventory) -> io::Result<()> {
    fs::remove_dir_all(&inventory.path)
}

pub(crate) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Sum of file sizes under `dir`, never following symlinks.
fn tree_bytes(dir: &Path) -> u64 {
    let mut total = 0;
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if meta.is_dir() {
            total += tree_bytes(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

/// `Some(pid)` when `<dir>/owner.lock` is held by a live process (`pid` is
/// `None` when its `endpoint.json` has none). The probe lock is released at
/// once.
fn live_owner(dir: &Path) -> Option<Option<u32>> {
    let lock = dir.join("owner.lock");
    if !fs::symlink_metadata(&lock).is_ok_and(|meta| meta.is_file()) {
        return None;
    }
    let file = fs::File::open(&lock).ok()?;
    match file.try_lock() {
        Err(fs::TryLockError::WouldBlock) => Some(descriptor_pid(&dir.join("endpoint.json"))),
        Ok(()) => {
            let _ = file.unlock();
            None
        }
        Err(_) => None,
    }
}

fn descriptor_pid(path: &Path) -> Option<u32> {
    use io::Read;
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(DESCRIPTOR_READ_LIMIT)
        .read_to_end(&mut bytes)
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    u32::try_from(value["pid"].as_u64()?).ok()
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
    fn scratch(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("ht-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    #[test]
    fn human_size_boundaries() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MiB");
    }

    #[test]
    fn scan_counts_sizes_and_does_not_follow_symlinks() {
        let state = scratch("legacy-scan");
        let instances = crate::daemon::paths::legacy_instances_dir(&state.0);
        fs::create_dir_all(instances.join("a")).unwrap();
        fs::create_dir_all(instances.join("b")).unwrap();
        fs::write(instances.join("a/threads.sqlite3"), [0u8; 100]).unwrap();
        fs::write(instances.join("b/settings.json"), [0u8; 20]).unwrap();
        let outside = scratch("legacy-scan-outside");
        fs::create_dir_all(&outside.0).unwrap();
        fs::write(outside.0.join("big"), vec![0u8; 5000]).unwrap();
        std::os::unix::fs::symlink(&outside.0, instances.join("a/link")).unwrap();
        let inventory = LegacyInventory::scan(&state.0).unwrap();
        assert_eq!(inventory.instances, 2);
        assert_eq!(inventory.settings, vec![instances.join("b/settings.json")]);
        assert!(inventory.live.is_empty());
        // 120 file bytes plus the link's own length; the 5000 outside bytes
        // are not counted.
        assert!(
            (120..1000).contains(&inventory.bytes),
            "{}",
            inventory.bytes
        );
    }

    #[test]
    fn absent_instances_is_none() {
        let state = scratch("legacy-absent");
        fs::create_dir_all(&state.0).unwrap();
        assert!(LegacyInventory::scan(&state.0).is_none());
    }

    #[test]
    fn held_lock_is_live_with_descriptor_pid_and_released_probe() {
        let state = scratch("legacy-live");
        let dir = crate::daemon::paths::legacy_instances_dir(&state.0).join("x");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("endpoint.json"), br#"{"pid": 4242}"#).unwrap();
        let held = fs::File::create(dir.join("owner.lock")).unwrap();
        held.lock().unwrap();
        let live = LegacyInventory::scan(&state.0).unwrap();
        assert_eq!(
            live.live,
            vec![LiveOwner {
                instance: "x".into(),
                pid: Some(4242)
            }]
        );
        assert_eq!(
            live.purge_preconditions(None, false, live.owner_uid),
            Err(PurgeRefusal::Live { pid: Some(4242) })
        );
        held.unlock().unwrap();
        // The probe released its own lock: it is free again, and not live.
        let free = LegacyInventory::scan(&state.0).unwrap();
        assert!(free.live.is_empty());
        let probe = fs::File::open(dir.join("owner.lock")).unwrap();
        assert!(probe.try_lock().is_ok());
    }

    #[test]
    fn preconditions_in_order() {
        let state = scratch("legacy-pre");
        fs::create_dir_all(crate::daemon::paths::legacy_instances_dir(&state.0)).unwrap();
        let inventory = LegacyInventory::scan(&state.0).unwrap();
        let uid = inventory.owner_uid;
        assert_eq!(inventory.purge_preconditions(None, false, uid), Ok(()));
        assert!(matches!(
            inventory.purge_preconditions(Some("marker".into()), false, uid),
            Err(PurgeRefusal::AgentEvidence(_))
        ));
        assert_eq!(
            inventory.purge_preconditions(Some("marker".into()), true, uid),
            Ok(())
        );
        assert_eq!(
            inventory.purge_preconditions(None, false, uid + 1),
            Err(PurgeRefusal::NotOwned { uid: uid + 1 })
        );
    }

    #[test]
    fn report_lines_render_every_kind_of_line() {
        let text = report_lines(&json!({
            "path": "/s/instances", "instances": 2, "bytes": 2048,
            "settings": ["/s/instances/a/settings.json"],
            "live": [{"instance": "a", "pid": 7}, {"instance": "b", "pid": null}],
            "remedy": REMEDY,
        }));
        assert_eq!(
            text,
            "legacy data: /s/instances (2 instances, 2.0 KiB) not used; remove: herdr-threads legacy purge\n\
             legacy daemon running: pid 7\n\
             legacy daemon running: pid unknown\n\
             legacy settings not carried over: /s/instances/a/settings.json\n"
        );
        assert_eq!(report_lines(&Value::Null), "");
    }
}
