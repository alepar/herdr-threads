//! Digest ownership for a small set of rendered files, with crash-recoverable publication.
//! Callers retain rendering, paths, metadata meaning, diagnostics and user-facing projections.
use crate::harness::{
    adapter::SetupFailure,
    setup::{fingerprint, write_replacement},
};
use crate::protocol::results::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
const MAX_BYTES: u64 = 1_048_576;

#[derive(Debug, Clone)]
pub struct Rendered {
    pub rel: &'static str,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub digest: String,
    /// An in-flight replacement's digest: the file may hold either until setup finalizes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<String>,
}
impl Entry {
    pub fn owns(&self, digest: &str) -> bool {
        self.digest == digest || self.pending.as_deref() == Some(digest)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest<M> {
    pub version: u8,
    pub config_root: String,
    pub binary_version: String,
    #[serde(flatten)]
    pub metadata: M,
    pub files: Vec<Entry>,
    pub created_dirs: Vec<String>,
}
fn conflict(detail: impl Into<String>) -> SetupFailure {
    SetupFailure::Api(ApiError::new(ErrorCode::Conflict, detail.into()))
}

pub struct FileSet<'a> {
    pub root: &'a Path,
    pub manifest: &'a Path,
    pub directories: &'static [(&'static str, bool)],
    pub invalid: fn(String) -> SetupFailure,
}
enum ReadFailure {
    Unsafe,
    TooLarge,
    Io(io::Error),
}
impl ReadFailure {
    fn describe(&self, path: &Path) -> String {
        match self {
            Self::Unsafe => format!("{} is a symlink or not a regular file", path.display()),
            Self::TooLarge => format!("{} exceeds the size bound", path.display()),
            Self::Io(error) => format!("could not read {}: {error}", path.display()),
        }
    }
}

/// A bounded, symlink-refusing read; `None` when the file is absent.
fn read_regular(path: &Path) -> Result<Option<Vec<u8>>, ReadFailure> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ReadFailure::Io(error)),
    };
    if !meta.file_type().is_file() {
        return Err(ReadFailure::Unsafe);
    }
    if meta.len() > MAX_BYTES {
        return Err(ReadFailure::TooLarge);
    }
    fs::read(path).map(Some).map_err(ReadFailure::Io)
}

pub fn read_manifest<M: DeserializeOwned>(
    path: &Path,
    invalid: fn(String) -> SetupFailure,
) -> Result<Option<Manifest<M>>, SetupFailure> {
    let bytes = match read_regular(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(None),
        Err(failure) => return Err(invalid(failure.describe(path))),
    };
    let manifest: Manifest<M> = serde_json::from_slice(&bytes)
        .map_err(|_| invalid(format!("{} is damaged", path.display())))?;
    if manifest.version != 1 {
        return Err(invalid(format!(
            "{} has an unknown version",
            path.display()
        )));
    }
    Ok(Some(manifest))
}

pub fn write_manifest<M: Serialize>(
    path: &Path,
    manifest: &Manifest<M>,
    invalid: fn(String) -> SetupFailure,
) -> Result<(), SetupFailure> {
    let bytes = serde_json::to_vec(manifest).map_err(|error| invalid(error.to_string()))?;
    write_replacement(path, &bytes, true)
        .map_err(|error| SetupFailure::Io(io::Error::other(format!("{error:?}"))))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    Created,
    Updated,
    Unchanged,
}
impl FileAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub files: Vec<(&'static str, FileAction)>,
    pub created_dirs: Vec<PathBuf>,
    pub metadata_changed: bool,
}
impl InstallReport {
    pub fn changed(&self) -> bool {
        self.metadata_changed
            || !self.created_dirs.is_empty()
            || self
                .files
                .iter()
                .any(|(_, action)| *action != FileAction::Unchanged)
    }
}

/// Make `path` a directory, creating missing ancestors outermost first. `strict` refuses a
/// symlink at `path` itself (setup's own `skills/herdr-threads`); shared directories such as
/// `plugins/` may be symlinked.
fn ensure_dir(path: &Path, strict: bool, created: &mut Vec<PathBuf>) -> Result<(), SetupFailure> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            let ok = if strict {
                meta.file_type().is_dir()
            } else {
                fs::metadata(path).is_ok_and(|meta| meta.is_dir())
            };
            if ok {
                Ok(())
            } else {
                Err(conflict(format!(
                    "{} exists but is not a plain directory; nothing was changed",
                    path.display()
                )))
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                ensure_dir(parent, false, created)?;
            }
            fs::create_dir(path).map_err(SetupFailure::Io)?;
            created.push(path.to_path_buf());
            Ok(())
        }
        Err(error) => Err(SetupFailure::Io(error)),
    }
}

impl FileSet<'_> {
    pub fn install<M: Serialize + DeserializeOwned + PartialEq>(
        &self,
        rendered: &[Rendered],
        metadata: M,
    ) -> Result<InstallReport, SetupFailure> {
        let root = self.root;
        let manifest_path = self.manifest;
        let recorded = read_manifest::<M>(manifest_path, self.invalid)?;
        if let Some(recorded) = &recorded
            && recorded.config_root != root.to_string_lossy()
        {
            return Err(conflict(format!(
                "{} records another config root; nothing was changed",
                manifest_path.display()
            )));
        }
        let mut plan = Vec::new();
        for file in rendered {
            let path = root.join(file.rel);
            let target = fingerprint(&file.bytes);
            let entry = recorded
                .as_ref()
                .and_then(|m| m.files.iter().find(|e| e.path == file.rel));
            let current = read_regular(&path).map_err(|failure| {
                conflict(format!("{}; nothing was changed", failure.describe(&path)))
            })?;
            // The digest of the bytes on disk, which may be the record's earlier pending one.
            let mut held = target.clone();
            let action = match current {
                None => FileAction::Created,
                Some(bytes) => match entry {
                    Some(entry) if entry.owns(&fingerprint(&bytes)) => {
                        if bytes == file.bytes {
                            FileAction::Unchanged
                        } else {
                            held = fingerprint(&bytes);
                            FileAction::Updated
                        }
                    }
                    _ => {
                        return Err(conflict(format!(
                            "{} was edited or is not owned by herdr-threads; it was not overwritten \
                         and nothing was changed. Move it aside or remove it, then set up again",
                            path.display()
                        )));
                    }
                },
            };
            plan.push((file, action, held, target));
        }
        let metadata_changed = recorded.as_ref().is_some_and(|m| m.metadata != metadata);
        let files_changed = plan
            .iter()
            .any(|(_, action, _, _)| *action != FileAction::Unchanged);
        let mut created_dirs = Vec::new();
        if files_changed || metadata_changed || recorded.is_none() {
            for (directory, strict) in self.directories {
                ensure_dir(&root.join(directory), *strict, &mut created_dirs)?;
            }
            let mut all_created = recorded
                .as_ref()
                .map(|m| m.created_dirs.clone())
                .unwrap_or_default();
            all_created.extend(
                created_dirs
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned()),
            );
            let mut manifest = Manifest {
                version: 1,
                config_root: root.to_string_lossy().into_owned(),
                binary_version: env!("CARGO_PKG_VERSION").into(),
                metadata,
                files: plan
                    .iter()
                    .map(|(file, _, held, target)| Entry {
                        path: file.rel.into(),
                        digest: held.clone(),
                        pending: (held != target).then(|| target.clone()),
                    })
                    .collect(),
                created_dirs: all_created,
            };
            // Intent first: a crash mid-write leaves files the manifest still recognizes.
            write_manifest(manifest_path, &manifest, self.invalid)?;
            for (file, action, _, _) in &plan {
                if *action != FileAction::Unchanged {
                    write_replacement(&root.join(file.rel), &file.bytes, true).map_err(
                        |error| SetupFailure::Io(io::Error::other(format!("{error:?}"))),
                    )?;
                }
            }
            for entry in &mut manifest.files {
                if let Some(pending) = entry.pending.take() {
                    entry.digest = pending;
                }
            }
            write_manifest(manifest_path, &manifest, self.invalid)?;
        }
        Ok(InstallReport {
            files: plan
                .iter()
                .map(|(file, action, _, _)| (file.rel, *action))
                .collect(),
            created_dirs,
            metadata_changed,
        })
    }
}
#[derive(Debug, Clone, Default)]
pub struct RemoveReport {
    pub removed: Vec<PathBuf>,
    pub residue: Vec<PathBuf>,
    /// A record existed.
    pub recorded: bool,
}

/// Remove only files that still match their recorded digest, then the directories setup created
/// that are now empty. An edited file is residue.
impl FileSet<'_> {
    pub fn remove<M: Serialize + DeserializeOwned>(&self) -> Result<RemoveReport, SetupFailure> {
        let root = self.root;
        let manifest_path = self.manifest;
        let Some(mut manifest) = read_manifest::<M>(manifest_path, self.invalid)? else {
            return Ok(RemoveReport::default());
        };
        if manifest.config_root != root.to_string_lossy() {
            return Err(conflict(format!(
                "{} records another config root; nothing was changed",
                manifest_path.display()
            )));
        }
        let mut report = RemoveReport {
            recorded: true,
            ..Default::default()
        };
        let mut kept = Vec::new();
        for entry in std::mem::take(&mut manifest.files) {
            let path = root.join(&entry.path);
            match read_regular(&path) {
                Ok(None) => {}
                Ok(Some(bytes)) if entry.owns(&fingerprint(&bytes)) => {
                    fs::remove_file(&path).map_err(SetupFailure::Io)?;
                    report.removed.push(path);
                }
                _ => {
                    report.residue.push(path);
                    kept.push(entry);
                }
            }
        }
        for dir in manifest.created_dirs.iter().rev() {
            // `remove_dir` refuses a non-empty directory: that is the emptiness check.
            let _ = fs::remove_dir(dir);
        }
        if kept.is_empty() {
            fs::remove_file(manifest_path).map_err(SetupFailure::Io)?;
        } else {
            manifest.files = kept;
            manifest.created_dirs.retain(|dir| Path::new(dir).exists());
            write_manifest(manifest_path, &manifest, self.invalid)?;
        }
        Ok(report)
    }
}
/// One owned file's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// No record (setup not run here).
    NotInstalled,
    /// No record, but a file sits at the owned path.
    Unowned,
    /// Recorded, but gone.
    Missing,
    /// Recorded, but its bytes differ from the record.
    Modified,
    /// Matches the record, but this build or instance renders other bytes.
    Stale,
    Installed,
}
impl FileState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::Unowned => "unowned",
            Self::Missing => "missing",
            Self::Modified => "modified",
            Self::Stale => "stale",
            Self::Installed => "installed",
        }
    }
}

pub fn inspect_file(
    root: &Path,
    rel: &str,
    entries: Option<&[Entry]>,
    expected: Option<&[Rendered]>,
) -> (FileState, Option<String>) {
    let bytes = read_regular(&root.join(rel));
    let digest = match &bytes {
        Ok(Some(bytes)) => Some(fingerprint(bytes)),
        _ => None,
    };
    let present = !matches!(bytes, Ok(None));
    let state = match entries {
        None if present => FileState::Unowned,
        None => FileState::NotInstalled,
        Some(entries) => match (entries.iter().find(|entry| entry.path == rel), &digest) {
            (_, None) if !present => FileState::Missing,
            (Some(entry), Some(digest)) if entry.pending.is_none() && entry.digest == *digest => {
                let target = expected
                    .and_then(|files| files.iter().find(|file| file.rel == rel))
                    .map(|file| fingerprint(&file.bytes));
                if target.as_deref() == Some(digest.as_str()) {
                    FileState::Installed
                } else {
                    FileState::Stale
                }
            }
            (None, _) if present => FileState::Unowned,
            _ => FileState::Modified,
        },
    };
    (state, digest)
}
