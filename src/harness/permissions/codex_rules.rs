//! The Codex permission component: `CODEX_HOME/rules/herdr-threads.rules`, a file herdr-threads
//! owns whole, holding the rendered execpolicy (see [`super::codex::render`]) and recorded in a
//! manifest of its own.
//!
//! Each publication records its intent first (target fingerprint, the file's fingerprint before,
//! the previous manifest), then writes or deletes the file (a changed or removed file is backed
//! up first), then clears the intent; the next operation settles an interrupted one the same way
//! the Claude component does. A file at the owned name that the manifest does not record, or an
//! owned file someone edited, is never overwritten or deleted.
use super::{
    OwnedConfigWriteGuard, OwnedPermissionResource, PERMISSION_MANIFEST_VERSION,
    PendingPublication, PermissionBackend, PermissionComponentManifest, PermissionConsent,
    VerifiedConfigIdentity, codex::render,
};
use crate::harness::setup::{
    SetupError, config_bytes, publish_manifest, remove_user_config, write_replacement,
    write_user_config,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

/// The fingerprint recorded for an absent file.
const ABSENT: &str = "absent";
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexPermissionState {
    /// Not installed (a recorded file the user deleted included).
    Missing,
    /// The owned file holds exactly what was written.
    Installed,
    /// The owned file was edited since; it is left as it is.
    Edited,
    /// A file at the owned name that herdr-threads did not write.
    Foreign,
    /// A publication was interrupted; the next setup or unsetup settles it.
    Pending,
}

pub struct CodexPermissions {
    rules: PathBuf,
    manifest: PathBuf,
}

fn hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl CodexPermissions {
    /// `codex_home` is the effective `CODEX_HOME`; `manifest` this component's private manifest.
    pub fn new(codex_home: &Path, manifest: PathBuf) -> Self {
        Self {
            rules: codex_home.join("rules").join("herdr-threads.rules"),
            manifest,
        }
    }

    pub fn rules_file(&self) -> &Path {
        &self.rules
    }

    fn current(&self) -> Result<Option<Vec<u8>>, SetupError> {
        match self.rules.symlink_metadata() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            _ => config_bytes(&self.rules).map(Some),
        }
    }

    fn fingerprint(&self) -> Result<String, SetupError> {
        Ok(self
            .current()?
            .map_or_else(|| ABSENT.to_owned(), |bytes| hex(&bytes)))
    }

    fn read(&self) -> Result<Option<PermissionComponentManifest>, SetupError> {
        let bytes = match fs::read(&self.manifest) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(SetupError::Io),
            Ok(bytes) => bytes,
        };
        let manifest: PermissionComponentManifest =
            serde_json::from_slice(&bytes).map_err(|_| SetupError::Invalid)?;
        if manifest.version != PERMISSION_MANIFEST_VERSION
            || manifest.backend != PermissionBackend::Codex
            || manifest.resources.len() > 1
            || manifest.resources.iter().any(|r| {
                r.rule.is_some() || r.pre_existing || r.file.file_name() != self.rules.file_name()
            })
        {
            return Err(SetupError::Conflict);
        }
        Ok(Some(manifest))
    }

    fn write(&self, manifest: &PermissionComponentManifest) -> Result<(), SetupError> {
        let bytes = serde_json::to_vec(manifest).map_err(|_| SetupError::Invalid)?;
        write_replacement(&self.manifest, &bytes, true)
    }

    fn remove_manifest(&self) -> Result<(), SetupError> {
        match fs::remove_file(&self.manifest) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(SetupError::Io),
            _ => Ok(()),
        }
    }

    fn lock(&self) -> Result<OwnedConfigWriteGuard, SetupError> {
        let home = self
            .rules
            .parent()
            .and_then(Path::parent)
            .ok_or(SetupError::Invalid)?
            .canonicalize()
            .map_err(|_| SetupError::Io)?;
        let identity = VerifiedConfigIdentity::verify(&home, PermissionBackend::Codex)
            .map_err(|_| SetupError::Invalid)?;
        OwnedConfigWriteGuard::acquire(&identity, LOCK_TIMEOUT).map_err(|_| SetupError::Io)
    }

    /// The recorded fingerprint of the owned file, when one is recorded.
    fn recorded(manifest: &PermissionComponentManifest) -> Option<&str> {
        manifest.resources.first().map(|r| r.fingerprint.as_str())
    }

    pub fn status(&self) -> Result<CodexPermissionState, SetupError> {
        let current = self.fingerprint()?;
        Ok(match self.read()? {
            Some(manifest) if manifest.pending.is_some() => CodexPermissionState::Pending,
            Some(manifest) => match Self::recorded(&manifest) {
                _ if current == ABSENT => CodexPermissionState::Missing,
                Some(recorded) if recorded == current => CodexPermissionState::Installed,
                _ => CodexPermissionState::Edited,
            },
            None if current == ABSENT => CodexPermissionState::Missing,
            None => CodexPermissionState::Foreign,
        })
    }

    fn settle(&self, _guard: &OwnedConfigWriteGuard) -> Result<(), SetupError> {
        let Some(mut manifest) = self.read()? else {
            return Ok(());
        };
        let Some(pending) = manifest.pending.take() else {
            return Ok(());
        };
        let current = self.fingerprint()?;
        if current == pending.after {
            if manifest.resources.is_empty() {
                self.remove_manifest()
            } else {
                self.write(&manifest)
            }
        } else if current == pending.before {
            match pending.previous {
                Some(previous) => self.write(&previous),
                None => self.remove_manifest(),
            }
        } else {
            Err(SetupError::Conflict)
        }
    }

    fn publish(
        &self,
        guard: &OwnedConfigWriteGuard,
        previous: Option<PermissionComponentManifest>,
        after: Option<&[u8]>,
        mut created: Vec<String>,
    ) -> Result<(), SetupError> {
        let current = self.current()?;
        let file = self.rules.clone();
        let mut intent = PermissionComponentManifest {
            version: PERMISSION_MANIFEST_VERSION,
            backend: PermissionBackend::Codex,
            config_root: file
                .parent()
                .and_then(Path::parent)
                .ok_or(SetupError::Invalid)?
                .into(),
            resources: after
                .map(|bytes| OwnedPermissionResource {
                    file,
                    rule: None,
                    fingerprint: hex(bytes),
                    pre_existing: false,
                })
                .into_iter()
                .collect(),
            created: Vec::new(),
            pending: Some(PendingPublication {
                before: current.as_deref().map_or_else(|| ABSENT.to_owned(), hex),
                after: after.map_or_else(|| ABSENT.to_owned(), hex),
                previous: previous.map(Box::new),
                retired: None,
            }),
        };
        let dir = self.rules.parent().ok_or(SetupError::Invalid)?;
        if after.is_some() && !dir.exists() {
            fs::create_dir(dir).map_err(|_| SetupError::Io)?;
            created.push("rules".into());
        }
        intent.created = created;
        self.write(&intent)?;
        #[cfg(test)]
        BEFORE_RULES_WRITE.with(|hook| hook.take().map(|hook| hook()));
        let previous = intent.pending.and_then(|pending| pending.previous);
        let written = match (current, after) {
            (Some(current), Some(after)) => write_user_config(&self.rules, &current, after),
            // Exclusive: a file created after the absent baseline is refused, never replaced.
            (None, Some(after)) => publish_manifest(&self.rules, after, false),
            (Some(current), None) => remove_user_config(&self.rules, &current),
            (None, None) => Ok(()),
        };
        if written == Err(SetupError::Conflict) {
            // The file changed before the write and was left alone: the intent never happened.
            match previous {
                Some(previous) => self.write(&previous)?,
                None => self.remove_manifest()?,
            }
            return Err(SetupError::Conflict);
        }
        written?;
        if after.is_none() && intent.created.iter().any(|c| c == "rules") {
            // Only an empty directory the component created goes.
            let _ = fs::remove_dir(dir);
        }
        self.settle(guard)
    }

    /// Install or refresh the owned file as far as `consent` allows: without consent only an
    /// installed file is kept current; a missing one stays missing.
    pub fn apply(&self, consent: PermissionConsent) -> Result<CodexPermissionState, SetupError> {
        let manifest = self.read()?;
        if consent != PermissionConsent::Granted && manifest.is_none() {
            return self.status();
        }
        let guard = self.lock()?;
        self.settle(&guard)?;
        let manifest = self.read()?;
        let current = self.fingerprint()?;
        let target = render().into_bytes();
        match &manifest {
            // A foreign or edited file is left alone: reported, refused only when the person
            // explicitly asked for the grant.
            None if current != ABSENT && consent == PermissionConsent::Granted => {
                return Err(SetupError::Conflict);
            }
            Some(m) if current != ABSENT && Self::recorded(m) != Some(current.as_str()) => {
                if consent == PermissionConsent::Granted {
                    return Err(SetupError::Conflict);
                }
                drop(guard);
                return self.status();
            }
            Some(_) if current == ABSENT && consent != PermissionConsent::Granted => {
                // The person deleted it: forget the record rather than re-grant.
                self.remove_manifest()?;
                return self.status();
            }
            _ if current == hex(&target) => return self.status(),
            _ => {}
        }
        let created = manifest
            .as_ref()
            .map(|m| m.created.clone())
            .unwrap_or_default();
        self.publish(&guard, manifest, Some(&target), created)?;
        drop(guard);
        self.status()
    }

    /// Remove the owned file (kept as a backup). An edited one is left in place and its record
    /// forgotten: the result is then `Edited`.
    pub fn remove(&self) -> Result<CodexPermissionState, SetupError> {
        if self.read()?.is_none() {
            return self.status();
        }
        let guard = self.lock()?;
        self.settle(&guard)?;
        let Some(manifest) = self.read()? else {
            drop(guard);
            return self.status();
        };
        let current = self.fingerprint()?;
        if current != ABSENT && Self::recorded(&manifest) != Some(current.as_str()) {
            self.remove_manifest()?;
            return Ok(CodexPermissionState::Edited);
        }
        let created = manifest.created.clone();
        self.publish(&guard, Some(manifest), None, created)?;
        drop(guard);
        self.status()
    }
}

#[cfg(test)]
thread_local! {
    /// Test hook: runs once after the intent is recorded, before the rules file is written.
    static BEFORE_RULES_WRITE: std::cell::Cell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
#[path = "../../../tests/harness/codex_permissions.rs"]
mod tests;
