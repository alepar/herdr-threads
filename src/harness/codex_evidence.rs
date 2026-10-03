//! The Codex hook's private, persistent admission state under the plugin state
//! directory:
//!
//! - `harness/codex-schema-cache.json`: the schema-fingerprint cache keyed by
//!   binary identity (owned by [`super::codex_schema`]; 0600, bounded), so a
//!   one-shot hook process does not rescan an unlisted Codex binary on every
//!   tool call;
//! - `harness/codex-hook-admission.json`: the stored admission evidence of the
//!   latest hook observation (listed, schema-matched live-unverified,
//!   optimistic, or refused), rewritten only when it changes.
//!
//! `harness/` is a 0700 directory the hook creates under an existing, owned
//! state root; it never creates the state root itself. Everything here is best
//! effort: a failure leaves the hook on its in-process cache and never blocks.
use super::codex::{InstalledAdmission, OPTIMISTIC_LABEL, SCHEMA_MATCHED_LABEL};
use std::{
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

/// Private subdirectory of the state root.
pub const DIR: &str = "harness";
const CACHE_FILE: &str = "codex-schema-cache.json";
const ADMISSION_FILE: &str = "codex-hook-admission.json";
/// Stored admission record bound (one evidence line plus a path).
const MAX_RECORD: u64 = 8 << 10;

/// `<state>/harness`.
pub fn dir(state_dir: &Path) -> PathBuf {
    state_dir.join(DIR)
}

/// `<state>/harness/codex-schema-cache.json`.
pub fn cache_path(private_dir: &Path) -> PathBuf {
    private_dir.join(CACHE_FILE)
}

/// `<state>/harness/codex-hook-admission.json`.
pub fn admission_path(private_dir: &Path) -> PathBuf {
    private_dir.join(ADMISSION_FILE)
}

/// Create (0700) or verify `<state>/harness` under an existing owned state
/// root. The state root is never created here.
pub fn prepare(state_dir: &Path) -> io::Result<PathBuf> {
    crate::daemon::paths::check_owned_state_root(state_dir)?;
    let private = dir(state_dir);
    crate::daemon::paths::ensure_private_dir(&private)?;
    Ok(private)
}

/// Verify an existing `<state>/harness` without creating anything.
pub fn existing(state_dir: &Path) -> io::Result<PathBuf> {
    crate::daemon::paths::check_owned_state_root(state_dir)?;
    let private = dir(state_dir);
    crate::daemon::paths::check_private_dir(&private)?;
    Ok(private)
}

/// The stored admission evidence of the latest hook observation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AdmissionRecord {
    pub format_version: u32,
    /// The absolute binary observed, if one was found on PATH.
    pub binary: Option<String>,
    /// `listed`, `schema-matched, live-unverified`, `optimistic`, `refused` or
    /// `not_found`.
    pub admission: String,
    /// [`InstalledAdmission::line`]: evidence or refusal summary.
    pub evidence: String,
    /// When this evidence was first recorded (Unix milliseconds).
    pub recorded_unix_ms: u64,
}

impl AdmissionRecord {
    pub fn of(admission: &InstalledAdmission, now_unix_ms: u64) -> Self {
        Self {
            format_version: 1,
            binary: admission
                .binary
                .as_ref()
                .map(|path| path.display().to_string()),
            admission: admission.state().to_owned(),
            evidence: admission.line(),
            recorded_unix_ms: now_unix_ms,
        }
    }

    /// True for a schema-matched (live-unverified) admission.
    pub fn schema_matched(&self) -> bool {
        self.admission == SCHEMA_MATCHED_LABEL
    }

    /// True for an optimistic (assumed-recipe, live-unverified) admission.
    pub fn optimistic(&self) -> bool {
        self.admission == OPTIMISTIC_LABEL
    }

    fn same_evidence(&self, other: &Self) -> bool {
        self.binary == other.binary
            && self.admission == other.admission
            && self.evidence == other.evidence
    }
}

/// Read the stored record: only a bounded private regular file this user
/// owns is trusted.
pub fn read(path: &Path) -> Option<AdmissionRecord> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file()
        || meta.len() > MAX_RECORD
        || meta.uid() != crate::daemon::paths::effective_uid()
        || meta.permissions().mode() & 0o077 != 0
    {
        return None;
    }
    let record: AdmissionRecord = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (record.format_version == 1).then_some(record)
}

/// Store `admission` as the latest hook evidence (0600, atomic rename). The
/// file is rewritten only when the evidence changed, so a steady state costs
/// one small read per hook. Returns whether it was written.
pub fn record(path: &Path, admission: &InstalledAdmission, now_unix_ms: u64) -> io::Result<bool> {
    let fresh = AdmissionRecord::of(admission, now_unix_ms);
    if read(path).is_some_and(|stored| stored.same_evidence(&fresh)) {
        return Ok(false);
    }
    let bytes = serde_json::to_vec(&fresh).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(io::Error::other("admission record exceeds its bound"));
    }
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut out| out.write_all(&bytes).and_then(|()| out.sync_all()))
        .and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written.map(|()| true)
}
