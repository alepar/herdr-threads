//! Owned installer skill files. A name or matching frontmatter is never ownership.
use super::{RunError, setup, skill};
use crate::{
    daemon::paths::{ensure_owned_state_root, ensure_private_dir},
    harness::{context::Harness, setup as owned},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    version: u8,
    harness: String,
    path: PathBuf,
    fingerprint: String,
    complete: bool,
}

pub(super) struct SkillFile {
    path: PathBuf,
    manifest: PathBuf,
    harness: &'static str,
    before: Option<Vec<u8>>,
    recorded: Option<Manifest>,
    manifest_before: Option<Vec<u8>>,
}

fn conflict(detail: impl Into<String>) -> RunError {
    RunError::Io(io::Error::other(detail.into()))
}

pub(super) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, RunError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() > 1_048_576 {
        return Err(conflict(format!(
            "{} is not a bounded regular file",
            path.display()
        )));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut bytes = Vec::new();
    options
        .open(path)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err(conflict("file exceeds size bound"));
    }
    Ok(Some(bytes))
}

fn directory(path: &Path) -> Result<(), RunError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        _ => Err(conflict(format!(
            "{} is not a regular directory",
            path.display()
        ))),
    }
}

impl SkillFile {
    pub(super) fn inspect(env: &setup::SetupEnv, harness: Harness) -> Result<Self, RunError> {
        // Destinations belong to these concrete supported harnesses, never guessed from IDs.
        let (name, root) = match harness {
            Harness::Claude => ("claude", env.claude_config_dir.as_deref()),
            Harness::Codex => ("codex", env.codex_home.as_deref()),
            Harness::Human => return Err(conflict("human panes have no skill installation")),
        };
        let root = root.ok_or_else(|| conflict(format!("{name} config root is unknown")))?;
        if !root.is_absolute() {
            return Err(conflict("skill config root must be absolute"));
        }
        let skills = root.join("skills");
        let folder = skills.join("herdr-threads");
        for dir in [root, &skills, &folder] {
            directory(dir)?;
        }
        let path = folder.join("SKILL.md");
        let manifest = setup::manifest_path(env.state_dir()?, &format!("{name}-skill"), &path);
        let before = read_optional(&path)?;
        let manifest_before = read_optional(&manifest)?;
        let recorded: Option<Manifest> = manifest_before
            .as_deref()
            .map(|bytes| {
                serde_json::from_slice(bytes)
                    .map_err(|_| conflict("invalid skill ownership manifest"))
            })
            .transpose()?;
        if let Some(recorded) = &recorded {
            if recorded.version != 1
                || recorded.harness != name
                || recorded.path != path
                || !recorded.complete
                || before.as_deref().map(owned::fingerprint).as_deref()
                    != Some(&recorded.fingerprint)
            {
                return Err(conflict(
                    "owned skill is partial or edited; preserved (restore the recorded file before updating)",
                ));
            }
        } else if before.is_some() || fs::symlink_metadata(&folder).is_ok() {
            return Err(conflict(format!(
                "unowned skill at {}; preserved",
                folder.display()
            )));
        }
        Ok(Self {
            path,
            manifest,
            harness: name,
            before,
            recorded,
            manifest_before,
        })
    }

    pub(super) fn installed(&self) -> bool {
        self.recorded.is_some()
    }

    pub(super) fn install(self, env: &setup::SetupEnv) -> Result<(), RunError> {
        let state = env.state_dir()?;
        // Canonical setup normally prepares this first. Skills remain independent of hooks.
        if let Some(parent) = state.parent() {
            fs::create_dir_all(parent)?;
        }
        ensure_owned_state_root(state)?;
        ensure_private_dir(&state.join("setup"))?;
        let folder = self.path.parent().expect("skill path has parent");
        for dir in [
            folder.parent().unwrap().parent().unwrap(),
            folder.parent().unwrap(),
            folder,
        ] {
            directory(dir)?;
        }
        if self.recorded.is_some() {
            directory(folder)?;
        } else {
            fs::create_dir_all(folder.parent().unwrap())?;
            fs::create_dir(folder).map_err(|error| {
                conflict(format!(
                    "skill directory appeared concurrently; preserved: {error}"
                ))
            })?;
        }
        if read_optional(&self.path)? != self.before {
            return Err(conflict("skill changed concurrently; preserved"));
        }
        if read_optional(&self.manifest)? != self.manifest_before {
            return Err(conflict("skill manifest changed concurrently; preserved"));
        }
        let bytes = skill::SKILL_MD.as_bytes();
        if self.before.as_deref() == Some(bytes) {
            return Ok(());
        }
        let mut manifest = Manifest {
            version: 1,
            harness: self.harness.into(),
            path: self.path.clone(),
            fingerprint: owned::fingerprint(bytes),
            complete: false,
        };
        let prepared =
            serde_json::to_vec(&manifest).map_err(|_| conflict("invalid skill manifest"))?;
        if self.recorded.is_some() {
            owned::write_replacement(&self.manifest, &prepared, true)
        } else {
            owned::publish_manifest(&self.manifest, &prepared, false)
        }
        .map_err(|error| conflict(format!("skill manifest publication failed: {error:?}")))?;
        // Prepared state is deliberately refused on later runs, never mistaken for owned install.
        if read_optional(&self.path)? != self.before {
            return Err(conflict(
                "skill changed concurrently after preparing update; preserved",
            ));
        }
        let publication = if self.before.is_some() {
            owned::write_replacement(&self.path, bytes, false)
        } else {
            owned::publish_manifest(&self.path, bytes, false)
        };
        publication.map_err(|error| {
            conflict(format!(
                "skill publication failed: {error:?}; inspect {}",
                self.manifest.display()
            ))
        })?;
        manifest.complete = true;
        owned::write_replacement(
            &self.manifest,
            &serde_json::to_vec(&manifest).unwrap(),
            true,
        )
        .map_err(|error| conflict(format!("skill ownership finalization failed: {error:?}")))?;
        Ok(())
    }
}
